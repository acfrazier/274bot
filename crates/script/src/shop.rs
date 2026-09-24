//! Rust-owned observed shop open/buy/sell/close sequencing.
//!
//! The selected caches own the shop interface identities (`shop_template`
//! 3824 / `shop_template:inv` 3900 for Buy, `shop_template_side` 3822 /
//! `shop_template_side:inv` 3823 for Sell) and their fixed op slots; the
//! compact snapshot seam owns where those containers currently are and what
//! they hold. JavaScript supplies the call argument, returns the caller's
//! completion and dispatches the verbs this module returns — the 10/5/1
//! batching, the packet-per-tick bound, the waits and the held-count
//! settlement policy stay here. A queued click is not a transfer: a batch is
//! only counted once the posted container counts moved.

use crate::observed::{self, ItemRow, Scene, Text};
use crate::task_clock::InstantTaskClock;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Frozen `Shop.open`: wait for `isOpen` this long per attempt.
pub const OPEN_WAIT_MS: u64 = 3_000;
/// Frozen `Shop.open` attempts.
pub const OPEN_ATTEMPTS: u32 = 3;
/// Frozen `Shop.close`: wait for `!isOpen` this long.
pub const CLOSE_WAIT_MS: u64 = 3_000;
/// Held-count settlement window before a batch counts as stalled.
pub const SETTLE_MS: u64 = 3_000;
/// Frozen batch bound: at most this many user-event packets in one tick.
pub const MAX_PACKETS_PER_TICK: usize = 5;

thread_local! {
    static RUNTIME: RefCell<ShopRuntime> = const { RefCell::new(ShopRuntime::new()) };
    static GAME_DATA: RefCell<Option<Arc<api::game_data::SelectedGameData>>> =
        const { RefCell::new(None) };
}

/// One posted container row (stock, shop player pack or backpack).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    name: Text,
    id: i32,
    slot: i32,
    component: i32,
    count: i32,
}

fn rows_of(rows: &[ItemRow]) -> Vec<Row> {
    rows.iter()
        .map(|row| Row {
            name: row.name.clone().unwrap_or_default(),
            id: row.id,
            slot: row.slot_or_unset(),
            component: row.component_or_unset(),
            count: row.count,
        })
        .collect()
}

/// The posted facts this module decides from, read from the isolate scene.
struct NativeObservation {
    ingame: bool,
    shop_open: bool,
    has_stock: bool,
    stock: Vec<Row>,
    /// `None` = the shop player pack was not decoded by the last rebuild.
    player: Option<Vec<Row>>,
    inv: Vec<Row>,
    npcs: Vec<(String, Vec<String>)>,
}

impl NativeObservation {
    /// A logout forgets the session: only pages posted since login count.
    /// The npc list is only read to open a shop.
    fn from_scene(scene: &Scene, with_npcs: bool) -> Self {
        let session = scene.since_login();
        Self {
            ingame: session.ingame().unwrap_or(false),
            shop_open: session.shop_open().unwrap_or(false),
            has_stock: session.shop_stock().is_some(),
            stock: session
                .shop_stock()
                .map(|rows| rows_of(rows))
                .unwrap_or_default(),
            player: session
                .shop_player()
                .and_then(Option::as_ref)
                .map(|rows| rows_of(rows)),
            inv: session.inv().map(|rows| rows_of(rows)).unwrap_or_default(),
            npcs: session
                .npcs()
                .filter(|_| with_npcs)
                .map(|rows| {
                    rows.iter()
                        .map(|npc| {
                            (
                                npc.name_or_empty().to_string(),
                                npc.actions
                                    .iter()
                                    .map(|action| action.to_string())
                                    .collect(),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    fn probe(&self) -> Probe<'_> {
        Probe {
            ingame: self.ingame,
            shop_open: self.shop_open,
            has_stock: self.has_stock,
            stock: &self.stock,
            player: self.player.as_deref(),
            inv: &self.inv,
            npcs: &self.npcs,
        }
    }
}

/// What the caller asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Open,
    Buy,
    Sell,
    Close,
}

/// Transfer phase of the current operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    /// Waiting for the posted open/close boundary.
    WaitBoundary,
    /// A batch of Buy/Sell ops is outstanding.
    WaitBatch,
    /// Settle one poll after a batch: the frozen `delayTicks(1)` that lets the
    /// batch's own server tick land before the recount.
    SettleTick,
}

struct ShopRuntime {
    clock: InstantTaskClock,
    token: u64,
    phase: Phase,
    kind: Kind,
    /// The selected row's name (Shop.open: the NPC name).
    name: String,
    /// The selected NPC's Trade op (Shop.open only).
    npc_action: String,
    /// Requested amount (`Buy`/`Sell`; 0 for open/close).
    requested: i32,
    /// Amount observed to have moved so far.
    transferred: i32,
    /// Observed amount of the outstanding batch (settled after one tick).
    batch_delta: i32,
    /// The observed held count when the outstanding batch was sent.
    batch_baseline: i32,
    /// The observed held count now (kept for the next batch baseline).
    held_now: i32,
    /// An op is outstanding in the current wait window (`Shop.open` presses
    /// one Trade per attempt, exactly like the frozen loop).
    pressed: bool,
    /// Remaining `Shop.open`/`Shop.close` attempts.
    attempts_left: u32,
}

impl ShopRuntime {
    const fn new() -> Self {
        Self {
            clock: InstantTaskClock::new(),
            token: 0,
            phase: Phase::Idle,
            kind: Kind::Buy,
            name: String::new(),
            npc_action: String::new(),
            requested: 0,
            transferred: 0,
            batch_delta: 0,
            batch_baseline: 0,
            held_now: 0,
            pressed: false,
            attempts_left: 0,
        }
    }

    fn frozen(&self) -> bool {
        self.clock.frozen()
    }

    fn set_freeze(&mut self, paused: bool, held: bool) {
        self.clock.set_freeze(paused, held);
    }

    fn arm(&mut self, window: u64) {
        self.clock.arm(window);
    }

    fn bound_reached(&self) -> bool {
        self.clock.bound_reached()
    }

    fn abort_runtime(&mut self) {
        self.token = self.token.wrapping_add(1);
        self.phase = Phase::Idle;
        self.name.clear();
        self.npc_action.clear();
        self.requested = 0;
        self.transferred = 0;
        self.batch_delta = 0;
        self.batch_baseline = 0;
        self.held_now = 0;
        self.pressed = false;
        self.attempts_left = 0;
        self.clock.deadline = None;
    }

    fn done(&mut self, result: bool, reason: &str) -> Value {
        let token = self.token;
        let quantity = self.transferred;
        self.phase = Phase::Idle;
        self.clock.deadline = None;
        json!({
            "kind": "done",
            "token": token,
            "result": result,
            "quantity": quantity,
            "reason": reason,
        })
    }

    fn npc_verb(&self) -> Value {
        json!({
            "kind": "npc",
            "token": self.token,
            "name": self.name,
            "action": self.npc_action,
        })
    }

    fn close_verb(&self) -> Value {
        json!({ "kind": "close-modal", "token": self.token })
    }

    /// Emit the next batch of Buy/Sell ops for the outstanding remainder and
    /// arm the held-count settlement window.
    fn send_batch(&mut self, row: &Row) -> Value {
        let remaining = self.requested - self.transferred;
        let chunks = plan(remaining);
        if chunks.is_empty() {
            return self.done(self.transferred >= self.requested, "requested");
        }
        let verb_kind = if self.kind == Kind::Buy {
            "buy"
        } else {
            "sell"
        };
        let ops: Vec<Value> = chunks
            .iter()
            .map(|chunk| {
                json!({
                    "op": "shop-button",
                    "kind": verb_kind,
                    "name": row.name,
                    "id": row.id,
                    "slot": row.slot,
                    "component": row.component,
                    "chunk": chunk,
                })
            })
            .collect();
        self.batch_delta = 0;
        self.batch_baseline = self.held_now;
        self.phase = Phase::WaitBatch;
        self.arm(SETTLE_MS);
        json!({ "kind": "ops", "token": self.token, "ops": ops })
    }
}

pub fn on_pause() {
    RUNTIME.with(|rt| {
        let held = rt.borrow().clock.held;
        rt.borrow_mut().set_freeze(true, held);
    });
}

pub fn on_resume() {
    RUNTIME.with(|rt| {
        let held = rt.borrow().clock.held;
        rt.borrow_mut().set_freeze(false, held);
    });
}

pub fn on_hold(held: bool) {
    RUNTIME.with(|rt| {
        let paused = rt.borrow().clock.paused;
        rt.borrow_mut().set_freeze(paused, held);
    });
}

pub fn on_reset() {
    RUNTIME.with(|rt| rt.borrow_mut().abort_runtime());
}

pub fn configure(data: Option<Arc<api::game_data::SelectedGameData>>) {
    GAME_DATA.with(|slot| *slot.borrow_mut() = data);
}

/// Open/buy/sell/close stay on the existing JSON shop binding. The planner
/// is an in-isolate Rust helper (typed request/result, native V8 marshalling),
/// not an extra shop JSON op and not a FlatBuffer RPC.
pub fn dispatch(_game_data: Option<&api::game_data::SelectedGameData>, input: &Value) -> Value {
    match input.get("op").and_then(Value::as_str).unwrap_or("") {
        "begin" => begin(input),
        "next" => next(input.get("token").and_then(Value::as_u64).unwrap_or(0)),
        _ => json!({ "kind": "notImpl", "reason": "unknown op" }),
    }
}

/// In-isolate buyout helper request. Built from V8 args; never a host wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BuyoutPlanRequest {
    pub inv: String,
    pub keeper: String,
    pub coins: i64,
    pub stock: Vec<(String, i32)>,
    pub chosen: Vec<String>,
}

/// One planned purchase row returned to JS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BuyoutPlanItem {
    pub obj: String,
    pub name: String,
    pub units: i32,
    pub est_cost: i64,
}

/// In-isolate buyout helper result. Materialized into a V8 object; never a host wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BuyoutPlanResult {
    pub ok: bool,
    pub reason: String,
    pub items: Vec<BuyoutPlanItem>,
}

/// Rust-owned buyout selection/ranking/stock-price/budget. JS only marshals
/// primitives into this in-isolate helper.
pub fn run_buyout_plan(req: &BuyoutPlanRequest) -> BuyoutPlanResult {
    GAME_DATA.with(|slot| {
        let data = slot.borrow();
        let Some(data) = data.as_deref() else {
            return BuyoutPlanResult {
                ok: false,
                reason: "missing shop facts".into(),
                items: Vec::new(),
            };
        };
        let shop = if !req.inv.is_empty() {
            match api::shop_facts::shop_by_inv(data, &req.inv) {
                Some(shop) => shop,
                None => {
                    return BuyoutPlanResult {
                        ok: false,
                        reason: format!("unsupported shop {}", req.inv),
                        items: Vec::new(),
                    }
                }
            }
        } else if !req.keeper.is_empty() {
            match api::shop_facts::shop_by_keeper(data, &req.keeper) {
                Some(shop) => shop,
                None => {
                    return BuyoutPlanResult {
                        ok: false,
                        reason: "unknown shopkeeper".into(),
                        items: Vec::new(),
                    }
                }
            }
        } else {
            return BuyoutPlanResult {
                ok: false,
                reason: "missing shop identity".into(),
                items: Vec::new(),
            };
        };
        let stock: HashMap<String, i32> = req.stock.iter().cloned().collect();
        let chosen: HashSet<String> = req.chosen.iter().cloned().collect();
        let items = api::shop_facts::buyout_plan(&shop, &stock, req.coins, &chosen);
        BuyoutPlanResult {
            ok: true,
            reason: String::new(),
            items: items
                .into_iter()
                .map(|row| BuyoutPlanItem {
                    obj: row.obj,
                    name: row.name,
                    units: row.units,
                    est_cost: row.est_cost,
                })
                .collect(),
        }
    })
}

/// The compact view one decision is made from.
struct Probe<'a> {
    ingame: bool,
    shop_open: bool,
    has_stock: bool,
    stock: &'a [Row],
    player: Option<&'a [Row]>,
    inv: &'a [Row],
    npcs: &'a [(String, Vec<String>)],
}

/// A terminal outcome for a transfer that never selected a row: refused
/// before any packet, so nothing moved.
fn refused(reason: &str) -> Value {
    RUNTIME.with(|rt| {
        let mut rt = rt.borrow_mut();
        rt.abort_runtime();
        let token = rt.token;
        json!({
            "kind": "done",
            "token": token,
            "result": false,
            "quantity": 0,
            "reason": reason,
        })
    })
}

fn begin(input: &Value) -> Value {
    let kind = match input.get("kind").and_then(Value::as_str).unwrap_or("") {
        "open" => Kind::Open,
        "buy" => Kind::Buy,
        "sell" => Kind::Sell,
        "close" => Kind::Close,
        _ => return json!({ "kind": "notImpl", "reason": "unknown shop op" }),
    };
    let name = input
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let qty = input.get("qty");
    let obs = observed::with(|scene| NativeObservation::from_scene(scene, kind == Kind::Open));
    let probe = obs.probe();
    if !probe.ingame {
        return json!({ "kind": "aborted", "reason": "not ingame" });
    }
    match kind {
        Kind::Open => {
            // The frozen `Shop.open` returns false when no posted NPC offers
            // that name with a Trade op: that is an absent world object, not
            // a missing host capability, so it resolves false without a
            // packet instead of throwing.
            let Some((npc_name, action)) = trade_target(probe.npcs, &name) else {
                return refused("no-trade-target");
            };
            RUNTIME.with(|rt| {
                let mut rt = rt.borrow_mut();
                rt.abort_runtime();
                rt.kind = Kind::Open;
                rt.name = npc_name;
                rt.npc_action = action;
                if probe.shop_open {
                    // Already open: the frozen Shop.open reports success and
                    // sends no Trade press.
                    return rt.done(true, "open");
                }
                rt.attempts_left = OPEN_ATTEMPTS;
                rt.phase = Phase::WaitBoundary;
                rt.pressed = true;
                rt.arm(OPEN_WAIT_MS);
                rt.npc_verb()
            })
        }
        Kind::Close => RUNTIME.with(|rt| {
            let mut rt = rt.borrow_mut();
            rt.abort_runtime();
            rt.kind = Kind::Close;
            if !probe.shop_open {
                return rt.done(true, "closed");
            }
            rt.phase = Phase::WaitBoundary;
            rt.arm(CLOSE_WAIT_MS);
            rt.close_verb()
        }),
        Kind::Buy | Kind::Sell => {
            if !probe.shop_open {
                return refused("closed");
            }
            if name.is_empty() {
                return refused("invalid");
            }
            let container = match kind {
                Kind::Buy => {
                    if !probe.has_stock {
                        return json!({ "kind": "notImpl", "reason": "missing shop stock" });
                    }
                    probe.stock
                }
                _ => {
                    let Some(player) = probe.player else {
                        return json!({ "kind": "notImpl", "reason": "missing shop player pack" });
                    };
                    player
                }
            };
            let Some(row) = find_row(container, &name).cloned() else {
                return refused("absent");
            };
            // The frozen `Shop.buy` breaks out on a stock row already at zero.
            if kind == Kind::Buy && row.count <= 0 {
                return refused("depleted");
            }
            let requested = match qty {
                Some(value) if value.as_i64().is_some_and(|qty| qty >= 1) => {
                    value.as_i64().unwrap_or(0) as i32
                }
                Some(value) if value.as_str() == Some("all") => row.count,
                _ => return refused("invalid"),
            };
            if requested < 1 {
                return refused("empty");
            }
            RUNTIME.with(|rt| {
                let mut rt = rt.borrow_mut();
                rt.abort_runtime();
                rt.kind = kind;
                rt.name = row.name.to_string();
                rt.requested = requested;
                rt.held_now = count_of(probe.inv, &row.name);
                rt.send_batch(&row)
            })
        }
    }
}

fn next(token: u64) -> Value {
    let obs = observed::with(|scene| NativeObservation::from_scene(scene, false));
    let probe = obs.probe();
    RUNTIME.with(|rt| {
        let mut rt = rt.borrow_mut();
        if token != rt.token || rt.phase == Phase::Idle {
            return json!({ "kind": "aborted", "token": rt.token });
        }
        if rt.frozen() {
            return json!({ "kind": "wait", "token": rt.token });
        }
        if !probe.ingame {
            let token = rt.token;
            rt.phase = Phase::Idle;
            rt.clock.deadline = None;
            return json!({ "kind": "aborted", "token": token });
        }
        match rt.kind {
            Kind::Open => open_step(&mut rt, &probe),
            Kind::Close => close_step(&mut rt, &probe),
            Kind::Buy | Kind::Sell => transfer_step(&mut rt, &probe),
        }
    })
}

fn open_step(rt: &mut ShopRuntime, probe: &Probe<'_>) -> Value {
    if probe.shop_open {
        return rt.done(true, "open");
    }
    // The frozen `Shop.open` presses Trade once per attempt and only re-presses
    // after that attempt's own 3000 ms window expired.
    if !rt.pressed {
        rt.pressed = true;
        rt.arm(OPEN_WAIT_MS);
        return rt.npc_verb();
    }
    if !rt.bound_reached() {
        return json!({ "kind": "wait", "token": rt.token });
    }
    rt.attempts_left = rt.attempts_left.saturating_sub(1);
    if rt.attempts_left == 0 {
        return rt.done(false, "open-timeout");
    }
    rt.pressed = true;
    rt.arm(OPEN_WAIT_MS);
    rt.npc_verb()
}

fn close_step(rt: &mut ShopRuntime, probe: &Probe<'_>) -> Value {
    if !probe.shop_open {
        return rt.done(true, "closed");
    }
    if rt.bound_reached() {
        return rt.done(false, "close-timeout");
    }
    json!({ "kind": "wait", "token": rt.token })
}

fn transfer_step(rt: &mut ShopRuntime, probe: &Probe<'_>) -> Value {
    if !probe.shop_open {
        return rt.done(false, "closed");
    }
    let container = match rt.kind {
        Kind::Buy => {
            if !probe.has_stock {
                return rt.done(false, "stock-missing");
            }
            probe.stock
        }
        _ => match probe.player {
            Some(player) => player,
            // The shop's player pack is our Sell identity: losing it stops
            // the transfer instead of falling back to the backpack.
            None => return rt.done(false, "player-missing"),
        },
    };
    let Some(row) = find_row(container, &rt.name).cloned() else {
        return rt.done(rt.transferred >= rt.requested, "depleted");
    };
    // The frozen `Shop.buy` breaks out on a stock row already at zero.
    if rt.kind == Kind::Buy && row.count <= 0 {
        return rt.done(rt.transferred >= rt.requested, "depleted");
    }
    match rt.phase {
        Phase::WaitBatch => {
            let delta = batch_delta(rt, probe);
            if delta > 0 {
                rt.batch_delta = delta;
                rt.phase = Phase::SettleTick;
                return json!({ "kind": "wait", "token": rt.token });
            }
            if rt.bound_reached() {
                // The window closed with no movement; the batch's own server
                // tick may still land, so settle exactly one more tick before
                // declaring the transfer stalled.
                rt.phase = Phase::SettleTick;
                return json!({ "kind": "wait", "token": rt.token });
            }
            json!({ "kind": "wait", "token": rt.token })
        }
        Phase::SettleTick => {
            let delta = batch_delta(rt, probe).max(rt.batch_delta);
            rt.transferred += delta;
            rt.held_now = count_of(probe.inv, &rt.name);
            if rt.transferred >= rt.requested {
                return rt.done(true, "done");
            }
            if delta == 0 {
                // Nothing moved across the settle window and its one tick.
                return rt.done(false, "stalled");
            }
            rt.send_batch(&row)
        }
        _ => rt.done(false, "aborted"),
    }
}

/// The items this batch moved, read from the posted backpack counts: Buy
/// adds to the pack, Sell takes from it.
fn batch_delta(rt: &ShopRuntime, probe: &Probe<'_>) -> i32 {
    let held = count_of(probe.inv, &rt.name);
    match rt.kind {
        Kind::Buy => (held - rt.batch_baseline).max(0),
        _ => (rt.batch_baseline - held).max(0),
    }
}

fn trade_target(npcs: &[(String, Vec<String>)], name: &str) -> Option<(String, String)> {
    let npc = npcs
        .iter()
        .find(|(npc_name, _)| npc_name.eq_ignore_ascii_case(name))?;
    let action = npc.1.iter().find(|a| a.eq_ignore_ascii_case("trade"))?;
    Some((npc.0.clone(), action.clone()))
}

fn find_row<'a>(rows: &'a [Row], name: &str) -> Option<&'a Row> {
    rows.iter().find(|row| row.name.eq_ignore_ascii_case(name))
}

fn count_of(rows: &[Row], name: &str) -> i32 {
    rows.iter()
        .filter(|row| row.name.eq_ignore_ascii_case(name))
        .map(|row| row.count)
        .sum()
}

/// The frozen 10/5/1 decomposition, capped at one tick's packet bound.
fn plan(remaining: i32) -> Vec<i32> {
    let mut chunks = Vec::new();
    let mut left = remaining;
    while left >= 10 && chunks.len() < MAX_PACKETS_PER_TICK {
        chunks.push(10);
        left -= 10;
    }
    if left >= 5 && chunks.len() < MAX_PACKETS_PER_TICK {
        chunks.push(5);
        left -= 5;
    }
    while left >= 1 && chunks.len() < MAX_PACKETS_PER_TICK {
        chunks.push(1);
        left -= 1;
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn row(name: &str, count: i32, slot: i32) -> Row {
        Row {
            name: name.into(),
            id: 221,
            slot,
            component: 3900,
            count,
        }
    }

    #[test]
    fn batching_is_the_frozen_ten_five_one_decomposition_capped_per_tick() {
        assert_eq!(plan(1), vec![1]);
        assert_eq!(plan(3), vec![1, 1, 1]);
        assert_eq!(plan(5), vec![5]);
        assert_eq!(plan(7), vec![5, 1, 1]);
        assert_eq!(plan(25), vec![10, 10, 5]);
        assert_eq!(plan(21), vec![10, 10, 1]);
        assert_eq!(plan(32), vec![10, 10, 10, 1, 1]);
        assert_eq!(plan(100), vec![10, 10, 10, 10, 10]);
    }

    #[test]
    fn plan_never_exceeds_the_tick_packet_bound_and_never_overshoots() {
        assert!(plan(0).is_empty());
        assert!(plan(-3).is_empty());
        for remaining in 1..=200 {
            let chunks = plan(remaining);
            assert!(chunks.len() <= MAX_PACKETS_PER_TICK, "{remaining}");
            assert!(chunks.iter().sum::<i32>() <= remaining);
        }
    }

    #[test]
    fn row_lookup_is_exact_by_name_against_the_posted_container() {
        let rows = [row("Eye of newt", 3, 5), row("Vial", 10, 6)];
        assert_eq!(find_row(&rows, "eye of newt").map(|r| r.count), Some(3));
        assert_eq!(find_row(&rows, "Vial").map(|r| r.slot), Some(6));
        assert!(find_row(&rows, "Feather").is_none());
        assert_eq!(count_of(&rows, "Vial"), 10);
        assert_eq!(count_of(&rows, "Feather"), 0);
    }

    #[test]
    fn count_of_aggregates_separate_unstackable_slots_without_unrelated_rows() {
        let rows = [
            row("Vial", 10, 1),
            row("Coins", 2000, 2),
            row("vIaL", 5, 3),
            row("Feather", 99, 4),
        ];
        assert_eq!(count_of(&rows, "VIAL"), 15);
        assert_eq!(count_of(&rows, "Coins"), 2000);
        assert_eq!(count_of(&rows, "Empty vial"), 0);
    }

    #[test]
    fn trade_target_needs_the_npc_and_its_trade_op() {
        let npcs = vec![(
            "Shop keeper".to_string(),
            vec!["Talk-to".to_string(), "Trade".to_string()],
        )];
        assert_eq!(
            trade_target(&npcs, "shop keeper"),
            Some(("Shop keeper".to_string(), "Trade".to_string()))
        );
        assert!(trade_target(&npcs, "Guard").is_none());
        let quiet = vec![("Guard".to_string(), vec!["Attack".to_string()])];
        assert!(trade_target(&quiet, "Guard").is_none());
        let no_npc = Vec::new();
        assert!(trade_target(&no_npc, "Shop keeper").is_none());
    }

    #[test]
    fn a_queued_batch_is_not_a_transfer_until_the_counts_move() {
        let mut rt = ShopRuntime::new();
        rt.kind = Kind::Buy;
        rt.name = "Feather".into();
        rt.requested = 10;
        rt.batch_baseline = 4;
        rt.phase = Phase::WaitBatch;
        rt.arm(SETTLE_MS);
        let inv = [row("Feather", 4, 1)];
        let stock = [row("Feather", 20, 3)];
        let probe = Probe {
            ingame: true,
            shop_open: true,
            has_stock: true,
            stock: &stock,
            player: None,
            inv: &inv,
            npcs: &[],
        };
        // Nothing moved yet: the batch still waits inside its window.
        let step = transfer_step(&mut rt, &probe);
        assert_eq!(step["kind"], "wait");
        assert_eq!(rt.transferred, 0);
        // Past the window the batch settles exactly one more tick (the tick
        // the frozen loop waits for its batch to land in) before it counts.
        rt.clock.deadline = Some(Instant::now() - Duration::from_millis(1));
        let step = transfer_step(&mut rt, &probe);
        assert_eq!(step["kind"], "wait");
        // Only a recount that is still empty stalls the transfer.
        let step = transfer_step(&mut rt, &probe);
        assert_eq!(step["kind"], "done");
        assert_eq!(step["result"], false);
        assert_eq!(step["quantity"], 0);
        assert_eq!(step["reason"], "stalled");
    }

    #[test]
    fn open_presses_trade_once_per_attempt_and_bounds_at_three() {
        let mut rt = ShopRuntime::new();
        rt.kind = Kind::Open;
        rt.name = "Shop keeper".into();
        rt.npc_action = "Trade".into();
        rt.attempts_left = OPEN_ATTEMPTS;
        rt.phase = Phase::WaitBoundary;
        rt.pressed = true;
        rt.arm(OPEN_WAIT_MS);
        let closed = [(
            "Shop keeper".to_string(),
            vec!["Talk-to".to_string(), "Trade".to_string()],
        )];
        let probe = Probe {
            ingame: true,
            shop_open: false,
            has_stock: false,
            stock: &[],
            player: None,
            inv: &[],
            npcs: &closed,
        };
        // Inside the attempt window the frozen loop only waits: no re-press.
        let step = open_step(&mut rt, &probe);
        assert_eq!(step["kind"], "wait");
        assert_eq!(rt.attempts_left, OPEN_ATTEMPTS);
        // Each expired window re-presses once, for the frozen three attempts.
        for attempt in 1..OPEN_ATTEMPTS {
            rt.clock.deadline = Some(Instant::now() - Duration::from_millis(1));
            let step = open_step(&mut rt, &probe);
            assert_eq!(step["kind"], "npc", "attempt {attempt} re-presses Trade");
            assert_eq!(step["action"], "Trade");
            assert_eq!(rt.attempts_left, OPEN_ATTEMPTS - attempt);
            let step = open_step(&mut rt, &probe);
            assert_eq!(step["kind"], "wait", "one press per window");
        }
        // The last window closes the call as a timeout, not another press.
        rt.clock.deadline = Some(Instant::now() - Duration::from_millis(1));
        let step = open_step(&mut rt, &probe);
        assert_eq!(step["kind"], "done");
        assert_eq!(step["result"], false);
        assert_eq!(step["reason"], "open-timeout");
    }

    #[test]
    fn open_reports_success_once_the_posted_shop_is_up() {
        let mut rt = ShopRuntime::new();
        rt.kind = Kind::Open;
        rt.attempts_left = OPEN_ATTEMPTS;
        rt.pressed = true;
        rt.phase = Phase::WaitBoundary;
        rt.arm(OPEN_WAIT_MS);
        let open = Probe {
            ingame: true,
            shop_open: true,
            has_stock: true,
            stock: &[],
            player: None,
            inv: &[],
            npcs: &[],
        };
        let step = open_step(&mut rt, &open);
        assert_eq!(step["kind"], "done");
        assert_eq!(step["result"], true);
        assert_eq!(step["reason"], "open");
    }

    #[test]
    fn a_depleted_stock_row_ends_buy_with_the_partial_count() {
        let mut rt = ShopRuntime::new();
        rt.kind = Kind::Buy;
        rt.name = "Feather".into();
        rt.requested = 25;
        rt.transferred = 10;
        rt.phase = Phase::SettleTick;
        rt.batch_delta = 0;
        let empty = [row("Feather", 0, 3)];
        let inv = [row("Feather", 12, 1)];
        let probe = Probe {
            ingame: true,
            shop_open: true,
            has_stock: true,
            stock: &empty,
            player: None,
            inv: &inv,
            npcs: &[],
        };
        let step = transfer_step(&mut rt, &probe);
        assert_eq!(step["kind"], "done");
        assert_eq!(step["result"], false, "the request was not filled");
        assert_eq!(step["quantity"], 10, "partial progress is reported");
        assert_eq!(step["reason"], "depleted");
    }

    #[test]
    fn a_served_batch_settles_one_tick_then_continues_the_remainder() {
        let mut rt = ShopRuntime::new();
        rt.kind = Kind::Buy;
        rt.name = "Feather".into();
        rt.requested = 25;
        rt.transferred = 10;
        rt.batch_baseline = 10;
        rt.phase = Phase::WaitBatch;
        rt.arm(SETTLE_MS);
        let stock = [row("Feather", 90, 3)];
        let probes_inv = [row("Feather", 20, 1)];
        let probe = Probe {
            ingame: true,
            shop_open: true,
            has_stock: true,
            stock: &stock,
            player: None,
            inv: &probes_inv,
            npcs: &[],
        };
        // The batch moved 10 of the 10 requested: settle one tick, no count yet.
        let step = transfer_step(&mut rt, &probe);
        assert_eq!(step["kind"], "wait");
        assert_eq!(rt.transferred, 10, "settlement is not added yet");

        // One tick later the delta is added and the remainder is batch-planned.
        let step = transfer_step(&mut rt, &probe);
        assert_eq!(rt.transferred, 20);
        assert_eq!(step["kind"], "ops");
        let chunks = step["ops"]
            .as_array()
            .unwrap()
            .iter()
            .map(|op| op["chunk"].as_i64().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(chunks, vec![5], "remaining 5 is the frozen 5 chunk");
        assert!(step["ops"]
            .as_array()
            .unwrap()
            .iter()
            .all(|op| op["op"] == "shop-button" && op["kind"] == "buy"));
    }

    #[test]
    fn buy_settles_total_across_separate_matching_inventory_slots() {
        let mut rt = ShopRuntime::new();
        rt.kind = Kind::Buy;
        rt.name = "Vial".into();
        rt.requested = 15;
        rt.batch_baseline = 0;
        rt.phase = Phase::WaitBatch;
        rt.arm(SETTLE_MS);
        let stock = [row("Vial", 20, 3), row("Feather", 99, 4)];
        let inv = [
            row("Vial", 10, 1),
            row("Coins", 1900, 2),
            row("vIaL", 5, 7),
            row("Feather", 99, 8),
        ];
        let probe = Probe {
            ingame: true,
            shop_open: true,
            has_stock: true,
            stock: &stock,
            player: None,
            inv: &inv,
            npcs: &[],
        };
        assert_eq!(transfer_step(&mut rt, &probe)["kind"], "wait");
        let settled = transfer_step(&mut rt, &probe);
        assert_eq!(settled["kind"], "done");
        assert_eq!(settled["result"], true);
        assert_eq!(settled["quantity"], 15);
        assert_eq!(rt.transferred, 15);
    }

    #[test]
    fn sell_plans_against_the_shop_player_pack_and_counts_inventory_losses() {
        let mut rt = ShopRuntime::new();
        rt.kind = Kind::Sell;
        rt.name = "Vial".into();
        rt.requested = 10;
        rt.batch_baseline = 12;
        rt.phase = Phase::SettleTick;
        rt.batch_delta = 10;
        let player = [row("Vial", 2, 4)];
        let inv = [row("Vial", 2, 8)];
        let probe = Probe {
            ingame: true,
            shop_open: true,
            has_stock: true,
            stock: &[],
            player: Some(&player),
            inv: &inv,
            npcs: &[],
        };
        let step = transfer_step(&mut rt, &probe);
        assert_eq!(rt.transferred, 10);
        assert_eq!(step["kind"], "done");
        assert_eq!(step["result"], true);
        assert_eq!(step["quantity"], 10);
    }

    #[test]
    fn sell_settles_total_across_separate_matching_inventory_slots() {
        let mut rt = ShopRuntime::new();
        rt.kind = Kind::Sell;
        rt.name = "Vial".into();
        rt.requested = 10;
        rt.batch_baseline = 20;
        rt.phase = Phase::WaitBatch;
        rt.arm(SETTLE_MS);
        let player = [row("Vial", 7, 4), row("Coins", 2000, 5), row("vIaL", 3, 9)];
        let inv = [
            row("Vial", 4, 8),
            row("Feather", 99, 10),
            row("vIaL", 6, 11),
        ];
        let probe = Probe {
            ingame: true,
            shop_open: true,
            has_stock: true,
            stock: &[],
            player: Some(&player),
            inv: &inv,
            npcs: &[],
        };
        assert_eq!(transfer_step(&mut rt, &probe)["kind"], "wait");
        let settled = transfer_step(&mut rt, &probe);
        assert_eq!(settled["kind"], "done");
        assert_eq!(settled["result"], true);
        assert_eq!(settled["quantity"], 10);
        assert_eq!(rt.transferred, 10);
    }

    #[test]
    fn losing_the_shop_player_pack_stops_sell_without_touching_the_backpack() {
        let mut rt = ShopRuntime::new();
        rt.kind = Kind::Sell;
        rt.name = "Vial".into();
        rt.requested = 5;
        rt.phase = Phase::WaitBatch;
        rt.arm(SETTLE_MS);
        let inv = [row("Vial", 9, 8)];
        let probe = Probe {
            ingame: true,
            shop_open: true,
            has_stock: true,
            stock: &[],
            player: None,
            inv: &inv,
            npcs: &[],
        };
        let step = transfer_step(&mut rt, &probe);
        assert_eq!(step["kind"], "done");
        assert_eq!(step["result"], false);
        assert_eq!(step["reason"], "player-missing");
    }

    #[test]
    fn pause_and_hold_freeze_the_settlement_deadline() {
        let mut rt = ShopRuntime::new();
        rt.arm(SETTLE_MS);
        let before = rt.clock.deadline.expect("armed");
        rt.set_freeze(true, false);
        assert!(rt.frozen());
        std::thread::sleep(Duration::from_millis(5));
        rt.set_freeze(false, false);
        assert!(!rt.frozen());
        assert!(rt.clock.deadline.expect("still armed") > before);
        rt.set_freeze(false, true);
        assert!(rt.frozen(), "guardian hold freezes too");
    }

    #[test]
    fn abort_runtime_bumps_the_token_and_clears_the_operation() {
        let mut rt = ShopRuntime::new();
        rt.kind = Kind::Buy;
        rt.name = "Feather".into();
        rt.requested = 25;
        rt.transferred = 10;
        rt.attempts_left = 3;
        rt.arm(SETTLE_MS);
        let before = rt.token;
        rt.abort_runtime();
        assert_eq!(rt.token, before.wrapping_add(1));
        assert_eq!(rt.phase, Phase::Idle);
        assert_eq!(rt.requested, 0);
        assert_eq!(rt.transferred, 0);
        assert_eq!(rt.attempts_left, 0);
        assert!(rt.clock.deadline.is_none());
        assert!(rt.name.is_empty());
    }
}
