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

use crate::isolate_fb::{RowReader, SnapshotReader};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::time::{Duration, Instant};

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
    static NATIVE_OBSERVATION: RefCell<NativeObservation> =
        const { RefCell::new(NativeObservation::new()) };
}

/// One posted container row (stock, shop player pack or backpack).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    name: String,
    id: i32,
    slot: i32,
    component: i32,
    count: i32,
}

fn rows_of(rows: Vec<RowReader<'_>>) -> Vec<Row> {
    rows.iter()
        .map(|row| Row {
            name: row.name().unwrap_or_default().to_string(),
            id: row.id(),
            slot: row.slot(),
            component: row.component_id(),
            count: row.count(),
        })
        .collect()
}

/// The compact projection this module keeps from posted snapshots. Deltas
/// only carry changed fields, so each field is remembered once seen.
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
    const fn new() -> Self {
        Self {
            ingame: false,
            shop_open: false,
            has_stock: false,
            stock: Vec::new(),
            player: None,
            inv: Vec::new(),
            npcs: Vec::new(),
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
        if snap.has_shop_open() {
            self.shop_open = snap.shop_open();
        }
        if snap.has_shop_stock() {
            self.has_stock = true;
            self.stock = rows_of(snap.shop_stock());
        }
        if snap.has_shop_player_available() {
            self.player = if snap.shop_player_available() {
                Some(rows_of(snap.shop_player()))
            } else {
                None
            };
        }
        if snap.has_inv() {
            self.inv = rows_of(snap.inv());
        }
        if snap.has_npcs() {
            self.npcs = snap
                .npcs()
                .iter()
                .map(|npc| {
                    (
                        npc.name().unwrap_or_default().to_string(),
                        npc.actions().iter().map(|a| a.to_string()).collect(),
                    )
                })
                .collect();
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
    paused: bool,
    held: bool,
    frozen_at: Option<Instant>,
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
    /// Remaining close attempts are bounded by the armed deadline.
    deadline: Option<Instant>,
}

impl ShopRuntime {
    const fn new() -> Self {
        Self {
            paused: false,
            held: false,
            frozen_at: None,
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
        self.npc_action.clear();
        self.requested = 0;
        self.transferred = 0;
        self.batch_delta = 0;
        self.batch_baseline = 0;
        self.held_now = 0;
        self.pressed = false;
        self.attempts_left = 0;
        self.deadline = None;
    }

    fn done(&mut self, result: bool, reason: &str) -> Value {
        let token = self.token;
        let quantity = self.transferred;
        self.phase = Phase::Idle;
        self.deadline = None;
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
    let obs = NATIVE_OBSERVATION.with(|o| {
        let o = o.borrow();
        (
            o.ingame,
            o.shop_open,
            o.has_stock,
            o.stock.clone(),
            o.player.clone(),
            o.inv.clone(),
            o.npcs.clone(),
        )
    });
    let probe = Probe {
        ingame: obs.0,
        shop_open: obs.1,
        has_stock: obs.2,
        stock: &obs.3,
        player: obs.4.as_deref(),
        inv: &obs.5,
        npcs: &obs.6,
    };
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
                rt.name = row.name.clone();
                rt.requested = requested;
                rt.held_now = count_of(probe.inv, &row.name);
                rt.send_batch(&row)
            })
        }
    }
}

fn next(token: u64) -> Value {
    NATIVE_OBSERVATION.with(|o| {
        let o = o.borrow();
        let probe = Probe {
            ingame: o.ingame,
            shop_open: o.shop_open,
            has_stock: o.has_stock,
            stock: &o.stock,
            player: o.player.as_deref(),
            inv: &o.inv,
            npcs: &o.npcs,
        };
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
                rt.deadline = None;
                return json!({ "kind": "aborted", "token": token });
            }
            match rt.kind {
                Kind::Open => open_step(&mut rt, &probe),
                Kind::Close => close_step(&mut rt, &probe),
                Kind::Buy | Kind::Sell => transfer_step(&mut rt, &probe),
            }
        })
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
        .find(|row| row.name.eq_ignore_ascii_case(name))
        .map_or(0, |row| row.count)
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

    fn row(name: &str, count: i32, slot: i32) -> Row {
        Row {
            name: name.to_string(),
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
        rt.deadline = Some(Instant::now() - Duration::from_millis(1));
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
            rt.deadline = Some(Instant::now() - Duration::from_millis(1));
            let step = open_step(&mut rt, &probe);
            assert_eq!(step["kind"], "npc", "attempt {attempt} re-presses Trade");
            assert_eq!(step["action"], "Trade");
            assert_eq!(rt.attempts_left, OPEN_ATTEMPTS - attempt);
            let step = open_step(&mut rt, &probe);
            assert_eq!(step["kind"], "wait", "one press per window");
        }
        // The last window closes the call as a timeout, not another press.
        rt.deadline = Some(Instant::now() - Duration::from_millis(1));
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
        assert!(rt.deadline.is_none());
        assert!(rt.name.is_empty());
    }
}
