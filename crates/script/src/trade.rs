//! Rust-owned player trade request, offer, remove, accept and decline.
//!
//! Frozen `Trade.ts` uses player op 4 (`Trade with`), exact side/mine
//! id+slot inv-buttons, Offer-X plus a real count-dialog, and posted
//! offer-versus-confirm buttons. JavaScript runs the optional pick
//! callback as a projection and dispatches the returned verbs — it does
//! not choose rows, wait the dialog, or treat a queued click as a
//! transfer. A noted identity, an over-offer, a stale screen or a
//! changed partner fail closed.

use crate::isolate_fb::{RowReader, SnapshotReader};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::time::{Duration, Instant};

/// Frozen `Input.interactPlayer` op slot: `set_player_op("Trade with", 4)`.
pub const TRADE_OP: usize = 4;
/// Frozen tradeside:inv component (`OFFER_INV`).
#[allow(dead_code)]
pub const OFFER_INV: i32 = 3322;
/// Frozen trademain:inv component (`MY_OFFER_INV`).
#[allow(dead_code)]
pub const MY_OFFER_INV: i32 = 3415;
/// Frozen tradeside option4 (`OFFER_ALL`).
pub const OFFER_ALL: i32 = 4;
/// Frozen tradeside option5 (`OFFER_X`).
pub const OFFER_X: i32 = 5;
/// Frozen trademain:inv option4 (`REMOVE_ALL`).
pub const REMOVE_ALL: i32 = 4;
/// Frozen count-dialog open wait.
pub const COUNT_OPEN_MS: u64 = 3_000;
/// Frozen offer settlement wait after Answer-Count.
pub const SETTLE_MS: u64 = 4_000;
/// Frozen decline-button close wait before falling back to close-modal.
pub const DECLINE_BUTTON_WAIT_MS: u64 = 1_200;
/// Frozen close-modal wait after a decline that left the screen up.
pub const DECLINE_CLOSE_MS: u64 = 3_000;
/// Frozen `removeAll` iteration cap.
pub const REMOVE_MAX: u32 = 28;
/// Per-removal progress window (one game tick in the frozen loop).
pub const REMOVE_STEP_MS: u64 = 600;

thread_local! {
    static RUNTIME: RefCell<TradeRuntime> = const { RefCell::new(TradeRuntime::new()) };
    static NATIVE_OBSERVATION: RefCell<NativeObservation> =
        const { RefCell::new(NativeObservation::new()) };
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    name: String,
    id: i32,
    slot: i32,
    component: i32,
    count: i32,
    noted: bool,
}

fn rows_of(rows: Vec<RowReader<'_>>) -> Vec<Row> {
    rows.iter()
        .map(|row| Row {
            name: row.name().unwrap_or_default().to_string(),
            id: row.id(),
            slot: row.slot(),
            component: row.component_id(),
            count: row.count(),
            noted: row.noted(),
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlayerRef {
    name: String,
    distance: i32,
    actions: Vec<String>,
}

struct NativeObservation {
    ingame: bool,
    offer_open: bool,
    confirm_open: bool,
    partner: Option<String>,
    accept_id: i32,
    decline_id: i32,
    count_dialog_open: bool,
    side: Vec<Row>,
    mine: Vec<Row>,
    players: Vec<PlayerRef>,
}

impl NativeObservation {
    const fn new() -> Self {
        Self {
            ingame: false,
            offer_open: false,
            confirm_open: false,
            partner: None,
            accept_id: -1,
            decline_id: -1,
            count_dialog_open: false,
            side: Vec::new(),
            mine: Vec::new(),
            players: Vec::new(),
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
        if snap.has_trade_offer_open() {
            self.offer_open = snap.trade_offer_open();
        }
        if snap.has_trade_confirm_open() {
            self.confirm_open = snap.trade_confirm_open();
        }
        if snap.has_trade_partner() {
            self.partner = snap
                .trade_partner()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_string);
        }
        if snap.has_trade_accept_id() {
            self.accept_id = snap.trade_accept_id();
        }
        if snap.has_trade_decline_id() {
            self.decline_id = snap.trade_decline_id();
        }
        if snap.has_count_dialog_open() {
            self.count_dialog_open = snap.count_dialog_open();
        }
        if snap.has_trade_side() {
            self.side = rows_of(snap.trade_side());
        }
        if snap.has_trade_mine() {
            self.mine = rows_of(snap.trade_mine());
        }
        if snap.has_players() {
            self.players = snap
                .players()
                .iter()
                .map(|player| PlayerRef {
                    name: player.name().unwrap_or_default().to_string(),
                    distance: player.distance(),
                    actions: player
                        .actions()
                        .iter()
                        .map(|action| (*action).to_string())
                        .collect(),
                })
                .collect();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Request,
    OfferAll,
    Offer,
    RemoveAll,
    Accept,
    Decline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    /// Same-name candidates posted; waiting for the pick projection.
    WaitSelect,
    /// Offer-X sent; waiting `count_dialog_open`.
    WaitCountOpen,
    /// Answer-Count sent; waiting the named offer to land.
    WaitSettle,
    /// One Remove-All sent; waiting my-offer to shrink.
    WaitRemove,
    /// Decline button sent; waiting the screen to drop.
    WaitDecline,
    /// Close-modal sent after decline left the screen up.
    WaitDeclineClose,
}

struct TradeRuntime {
    paused: bool,
    held: bool,
    frozen_at: Option<Instant>,
    token: u64,
    phase: Phase,
    kind: Kind,
    name: String,
    player_action: String,
    selected: Option<Row>,
    amount: i32,
    mine_baseline: i32,
    mine_len: usize,
    partner: Option<String>,
    offer_open: bool,
    confirm_open: bool,
    removals: u32,
    deadline: Option<Instant>,
}

impl TradeRuntime {
    const fn new() -> Self {
        Self {
            paused: false,
            held: false,
            frozen_at: None,
            token: 0,
            phase: Phase::Idle,
            kind: Kind::Request,
            name: String::new(),
            player_action: String::new(),
            selected: None,
            amount: 0,
            mine_baseline: 0,
            mine_len: 0,
            partner: None,
            offer_open: false,
            confirm_open: false,
            removals: 0,
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
        self.name.clear();
        self.player_action.clear();
        self.selected = None;
        self.amount = 0;
        self.mine_baseline = 0;
        self.mine_len = 0;
        self.partner = None;
        self.offer_open = false;
        self.confirm_open = false;
        self.removals = 0;
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

    fn player_verb(&self) -> Value {
        json!({
            "kind": "ops",
            "token": self.token,
            "ops": [{
                "op": "player",
                "name": self.name,
                "action": self.player_action,
            }],
        })
    }

    fn inv_button(&self, row: &Row, operation: i32) -> Value {
        json!({
            "kind": "ops",
            "token": self.token,
            "ops": [{
                "op": "inv-button",
                "id": row.id,
                "slot": row.slot,
                "component": row.component,
                "operation": operation,
                "bank_generation": 0,
            }],
        })
    }

    fn if_button(&self, component_id: i32) -> Value {
        json!({
            "kind": "ops",
            "token": self.token,
            "ops": [{ "op": "if-button", "component_id": component_id }],
        })
    }

    fn answer_count(&self) -> Value {
        json!({
            "kind": "ops",
            "token": self.token,
            "ops": [{ "op": "answer-count", "value": self.amount }],
        })
    }

    fn close_verb(&self) -> Value {
        json!({
            "kind": "ops",
            "token": self.token,
            "ops": [{ "op": "close-modal" }],
        })
    }

    fn finish_ops(&mut self, mut step: Value, result: bool, reason: &str) -> Value {
        let token = self.token;
        self.phase = Phase::Idle;
        self.deadline = None;
        step["kind"] = json!("done");
        step["token"] = json!(token);
        step["result"] = json!(result);
        step["reason"] = json!(reason);
        step
    }

    fn candidates(&self, rows: &[Row]) -> Value {
        let candidates: Vec<Value> = rows
            .iter()
            .map(|row| {
                json!({
                    "id": row.id,
                    "slot": row.slot,
                    "count": row.count,
                    "noted": row.noted,
                })
            })
            .collect();
        json!({
            "kind": "candidates",
            "token": self.token,
            "candidates": candidates,
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
        "select" => select(input),
        "next" => next(input.get("token").and_then(Value::as_u64).unwrap_or(0)),
        _ => json!({ "kind": "notImpl", "reason": "unknown op" }),
    }
}

struct Probe<'a> {
    ingame: bool,
    offer_open: bool,
    confirm_open: bool,
    partner: Option<&'a str>,
    accept_id: i32,
    decline_id: i32,
    count_dialog_open: bool,
    side: &'a [Row],
    mine: &'a [Row],
    players: &'a [PlayerRef],
}

fn refused(reason: &str) -> Value {
    RUNTIME.with(|rt| {
        let mut rt = rt.borrow_mut();
        rt.abort_runtime();
        rt.done(false, reason)
    })
}

fn matches_name(row: &Row, name: &str) -> bool {
    !name.is_empty() && row.name.eq_ignore_ascii_case(name)
}

fn named_side<'a>(side: &'a [Row], name: &str) -> Vec<&'a Row> {
    side.iter().filter(|row| matches_name(row, name)).collect()
}

fn offered_of(mine: &[Row], id: i32) -> i32 {
    mine.iter()
        .filter(|row| row.id == id)
        .map(|row| row.count.max(1))
        .sum()
}

fn trade_action(player: &PlayerRef) -> Option<&str> {
    player
        .actions
        .get(TRADE_OP - 1)
        .map(String::as_str)
        .filter(|action| {
            let trimmed = action.trim();
            !trimmed.is_empty() && !trimmed.eq_ignore_ascii_case("hidden")
        })
}

fn nearest_named<'a>(players: &'a [PlayerRef], name: &str) -> Option<&'a PlayerRef> {
    let wanted = name.trim();
    if wanted.is_empty() {
        return None;
    }
    players
        .iter()
        .filter(|player| player.name.eq_ignore_ascii_case(wanted))
        .min_by_key(|player| player.distance)
}

fn screen_stale(rt: &TradeRuntime, probe: &Probe<'_>) -> bool {
    if rt.offer_open != probe.offer_open || rt.confirm_open != probe.confirm_open {
        return true;
    }
    match (rt.partner.as_deref(), probe.partner) {
        (Some(want), Some(have)) => !want.eq_ignore_ascii_case(have),
        (Some(_), None) => true,
        (None, _) => false,
    }
}

fn begin(input: &Value) -> Value {
    let kind = match input.get("kind").and_then(Value::as_str).unwrap_or("") {
        "request" => Kind::Request,
        "offerAll" => Kind::OfferAll,
        "offer" => Kind::Offer,
        "removeAll" => Kind::RemoveAll,
        "accept" => Kind::Accept,
        "decline" => Kind::Decline,
        _ => return json!({ "kind": "notImpl", "reason": "unknown trade op" }),
    };
    let name = input
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let n = input.get("n").and_then(Value::as_i64).unwrap_or(0);
    let obs = NATIVE_OBSERVATION.with(|o| {
        let o = o.borrow();
        (
            o.ingame,
            o.offer_open,
            o.confirm_open,
            o.partner.clone(),
            o.accept_id,
            o.decline_id,
            o.count_dialog_open,
            o.side.clone(),
            o.mine.clone(),
            o.players.clone(),
        )
    });
    let probe = Probe {
        ingame: obs.0,
        offer_open: obs.1,
        confirm_open: obs.2,
        partner: obs.3.as_deref(),
        accept_id: obs.4,
        decline_id: obs.5,
        count_dialog_open: obs.6,
        side: &obs.7,
        mine: &obs.8,
        players: &obs.9,
    };
    if !probe.ingame {
        return json!({ "kind": "aborted", "reason": "not ingame" });
    }
    match kind {
        Kind::Request => {
            let Some(player) = nearest_named(probe.players, &name) else {
                return refused("no-player");
            };
            let Some(action) = trade_action(player) else {
                return refused("no-trade-op");
            };
            RUNTIME.with(|rt| {
                let mut rt = rt.borrow_mut();
                rt.abort_runtime();
                rt.kind = Kind::Request;
                rt.name = player.name.clone();
                rt.player_action = action.to_string();
                let verb = rt.player_verb();
                rt.finish_ops(verb, true, "sent")
            })
        }
        Kind::OfferAll | Kind::Offer => {
            if !probe.offer_open {
                return refused("missing-screen");
            }
            if name.is_empty() {
                return refused("no-match");
            }
            if kind == Kind::Offer && n <= 0 {
                return refused("invalid");
            }
            let matches = named_side(probe.side, &name);
            if matches.is_empty() {
                return refused("no-match");
            }
            RUNTIME.with(|rt| {
                let mut rt = rt.borrow_mut();
                rt.abort_runtime();
                rt.kind = kind;
                rt.name = name;
                rt.amount = n as i32;
                rt.partner = probe.partner.map(str::to_string);
                rt.offer_open = probe.offer_open;
                rt.confirm_open = probe.confirm_open;
                rt.phase = Phase::WaitSelect;
                let rows: Vec<Row> = matches.into_iter().cloned().collect();
                rt.candidates(&rows)
            })
        }
        Kind::RemoveAll => {
            if !probe.offer_open {
                return refused("missing-screen");
            }
            if probe.mine.is_empty() {
                return RUNTIME.with(|rt| {
                    let mut rt = rt.borrow_mut();
                    rt.abort_runtime();
                    rt.done(true, "empty")
                });
            }
            let first = probe.mine[0].clone();
            if first.component < 0 || first.slot < 0 {
                return refused("no-identity");
            }
            RUNTIME.with(|rt| {
                let mut rt = rt.borrow_mut();
                rt.abort_runtime();
                rt.kind = Kind::RemoveAll;
                rt.partner = probe.partner.map(str::to_string);
                rt.offer_open = true;
                rt.confirm_open = false;
                rt.selected = Some(first.clone());
                rt.mine_len = probe.mine.len();
                rt.removals = 1;
                rt.phase = Phase::WaitRemove;
                rt.arm(REMOVE_STEP_MS);
                rt.inv_button(&first, REMOVE_ALL)
            })
        }
        Kind::Accept => {
            if probe.confirm_open {
                if probe.accept_id < 0 {
                    return refused("no-accept");
                }
                return RUNTIME.with(|rt| {
                    let mut rt = rt.borrow_mut();
                    rt.abort_runtime();
                    let verb = rt.if_button(probe.accept_id);
                    rt.finish_ops(verb, true, "sent")
                });
            }
            if probe.offer_open {
                if probe.accept_id < 0 {
                    return refused("no-accept");
                }
                return RUNTIME.with(|rt| {
                    let mut rt = rt.borrow_mut();
                    rt.abort_runtime();
                    let verb = rt.if_button(probe.accept_id);
                    rt.finish_ops(verb, true, "sent")
                });
            }
            refused("missing-screen")
        }
        Kind::Decline => {
            if !probe.offer_open && !probe.confirm_open {
                return RUNTIME.with(|rt| {
                    let mut rt = rt.borrow_mut();
                    rt.abort_runtime();
                    rt.done(true, "inactive")
                });
            }
            RUNTIME.with(|rt| {
                let mut rt = rt.borrow_mut();
                rt.abort_runtime();
                rt.kind = Kind::Decline;
                rt.partner = probe.partner.map(str::to_string);
                rt.offer_open = probe.offer_open;
                rt.confirm_open = probe.confirm_open;
                if probe.decline_id < 0 {
                    rt.phase = Phase::WaitDeclineClose;
                    rt.arm(DECLINE_CLOSE_MS);
                    return rt.close_verb();
                }
                rt.phase = Phase::WaitDecline;
                rt.arm(DECLINE_BUTTON_WAIT_MS);
                rt.if_button(probe.decline_id)
            })
        }
    }
}

fn select(input: &Value) -> Value {
    let token = input.get("token").and_then(Value::as_u64).unwrap_or(0);
    let id = input.get("id").and_then(Value::as_i64);
    let slot = input.get("slot").and_then(Value::as_i64);
    NATIVE_OBSERVATION.with(|o| {
        let o = o.borrow();
        let probe = Probe {
            ingame: o.ingame,
            offer_open: o.offer_open,
            confirm_open: o.confirm_open,
            partner: o.partner.as_deref(),
            accept_id: o.accept_id,
            decline_id: o.decline_id,
            count_dialog_open: o.count_dialog_open,
            side: &o.side,
            mine: &o.mine,
            players: &o.players,
        };
        RUNTIME.with(|rt| {
            let mut rt = rt.borrow_mut();
            if token != rt.token || rt.phase != Phase::WaitSelect {
                return json!({ "kind": "aborted", "token": rt.token });
            }
            if rt.frozen() {
                return rt.wait();
            }
            if !probe.ingame {
                return rt.done(false, "not-ingame");
            }
            if !probe.offer_open || screen_stale(&rt, &probe) {
                return rt.done(false, "stale-screen");
            }
            let Some(id) = id.and_then(|n| i32::try_from(n).ok()) else {
                return rt.done(false, "no-match");
            };
            let Some(slot) = slot.and_then(|n| i32::try_from(n).ok()) else {
                return rt.done(false, "no-match");
            };
            let Some(row) = named_side(probe.side, &rt.name)
                .into_iter()
                .find(|row| row.id == id && row.slot == slot)
                .cloned()
            else {
                return rt.done(false, "wrong-identity");
            };
            if row.noted {
                return rt.done(false, "noted");
            }
            if row.component < 0 || row.slot < 0 {
                return rt.done(false, "no-identity");
            }
            match rt.kind {
                Kind::OfferAll => {
                    rt.selected = Some(row.clone());
                    let verb = rt.inv_button(&row, OFFER_ALL);
                    rt.finish_ops(verb, true, "sent")
                }
                Kind::Offer => {
                    if rt.amount > row.count {
                        return rt.done(false, "over-offer");
                    }
                    rt.selected = Some(row.clone());
                    rt.mine_baseline = offered_of(probe.mine, row.id);
                    rt.phase = Phase::WaitCountOpen;
                    rt.arm(COUNT_OPEN_MS);
                    rt.inv_button(&row, OFFER_X)
                }
                _ => rt.done(false, "wrong-kind"),
            }
        })
    })
}

fn next(token: u64) -> Value {
    NATIVE_OBSERVATION.with(|o| {
        let o = o.borrow();
        let probe = Probe {
            ingame: o.ingame,
            offer_open: o.offer_open,
            confirm_open: o.confirm_open,
            partner: o.partner.as_deref(),
            accept_id: o.accept_id,
            decline_id: o.decline_id,
            count_dialog_open: o.count_dialog_open,
            side: &o.side,
            mine: &o.mine,
            players: &o.players,
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
                return rt.done(false, "not-ingame");
            }
            match rt.kind {
                Kind::Offer => offer_step(&mut rt, &probe),
                Kind::RemoveAll => remove_step(&mut rt, &probe),
                Kind::Decline => decline_step(&mut rt, &probe),
                Kind::OfferAll | Kind::Request | Kind::Accept => rt.done(true, "sent"),
            }
        })
    })
}

fn offer_step(rt: &mut TradeRuntime, probe: &Probe<'_>) -> Value {
    if !probe.offer_open || screen_stale(rt, probe) {
        return rt.done(false, "stale-screen");
    }
    let Some(selected) = rt.selected.clone() else {
        return rt.done(false, "no-match");
    };
    match rt.phase {
        Phase::WaitCountOpen => {
            if probe.count_dialog_open {
                rt.phase = Phase::WaitSettle;
                rt.arm(SETTLE_MS);
                return rt.answer_count();
            }
            if rt.bound_reached() {
                return rt.done(false, "count-timeout");
            }
            rt.wait()
        }
        Phase::WaitSettle => {
            if offered_of(probe.mine, selected.id) >= rt.mine_baseline + rt.amount {
                return rt.done(true, "settled");
            }
            if rt.bound_reached() {
                return rt.done(false, "late");
            }
            rt.wait()
        }
        _ => rt.wait(),
    }
}

fn remove_step(rt: &mut TradeRuntime, probe: &Probe<'_>) -> Value {
    if !probe.offer_open || screen_stale(rt, probe) {
        return rt.done(false, "stale-screen");
    }
    if probe.mine.is_empty() {
        return rt.done(true, "empty");
    }
    if probe.mine.len() < rt.mine_len {
        if rt.removals >= REMOVE_MAX {
            return rt.done(probe.mine.is_empty(), "cap");
        }
        let next = probe.mine[0].clone();
        if next.component < 0 || next.slot < 0 {
            return rt.done(false, "no-identity");
        }
        rt.selected = Some(next.clone());
        rt.mine_len = probe.mine.len();
        rt.removals += 1;
        rt.arm(REMOVE_STEP_MS);
        return rt.inv_button(&next, REMOVE_ALL);
    }
    if rt.bound_reached() {
        return rt.done(false, "no-progress");
    }
    rt.wait()
}

fn decline_step(rt: &mut TradeRuntime, probe: &Probe<'_>) -> Value {
    let active = probe.offer_open || probe.confirm_open;
    match rt.phase {
        Phase::WaitDecline => {
            if !active {
                return rt.done(true, "closed");
            }
            if rt.bound_reached() {
                rt.phase = Phase::WaitDeclineClose;
                rt.arm(DECLINE_CLOSE_MS);
                return rt.close_verb();
            }
            rt.wait()
        }
        Phase::WaitDeclineClose => {
            if !active {
                return rt.done(true, "closed");
            }
            if rt.bound_reached() {
                return rt.done(false, "still-open");
            }
            rt.wait()
        }
        _ => rt.wait(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trade_op_is_the_frozen_fourth_player_slot() {
        assert_eq!(TRADE_OP, 4);
        let player = PlayerRef {
            name: "bob".into(),
            distance: 1,
            actions: vec![
                "Attack".into(),
                "Follow".into(),
                "Report".into(),
                "Trade with".into(),
            ],
        };
        assert_eq!(trade_action(&player), Some("Trade with"));
        let missing = PlayerRef {
            name: "bob".into(),
            distance: 1,
            actions: vec!["Attack".into(), "Follow".into()],
        };
        assert!(trade_action(&missing).is_none());
    }

    #[test]
    fn over_offer_is_refused_before_a_count_answer() {
        assert!(n_within_held(5, 4).is_none());
        assert_eq!(n_within_held(4, 4), Some(4));
        assert!(n_within_held(0, 4).is_none());
    }

    fn n_within_held(n: i32, held: i32) -> Option<i32> {
        if n <= 0 || held <= 0 || n > held {
            None
        } else {
            Some(n)
        }
    }

    #[test]
    fn pause_and_hold_freeze_the_settlement_deadline() {
        let mut rt = TradeRuntime::new();
        rt.arm(SETTLE_MS);
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
        let mut rt = TradeRuntime::new();
        rt.kind = Kind::Offer;
        rt.name = "Rune essence".into();
        rt.amount = 25;
        rt.arm(SETTLE_MS);
        let before = rt.token;
        rt.abort_runtime();
        assert_eq!(rt.token, before.wrapping_add(1));
        assert_eq!(rt.phase, Phase::Idle);
        assert_eq!(rt.amount, 0);
        assert!(rt.deadline.is_none());
        assert!(rt.name.is_empty());
        assert!(rt.selected.is_none());
    }

    #[test]
    fn offer_constants_match_frozen_trade_ts() {
        assert_eq!(OFFER_INV, 3322);
        assert_eq!(MY_OFFER_INV, 3415);
        assert_eq!(OFFER_ALL, 4);
        assert_eq!(OFFER_X, 5);
        assert_eq!(REMOVE_ALL, 4);
        assert_eq!(COUNT_OPEN_MS, 3_000);
        assert_eq!(SETTLE_MS, 4_000);
        assert_eq!(DECLINE_BUTTON_WAIT_MS, 1_200);
        assert_eq!(DECLINE_CLOSE_MS, 3_000);
        assert_eq!(REMOVE_MAX, 28);
    }
}
