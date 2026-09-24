//! Rust-owned observed shop open/buy/sell/close sequencing, the `shop`
//! [`crate::machine`] family.
//!
//! The selected caches own the shop interface identities (`shop_template`
//! 3824 / `shop_template:inv` 3900 for Buy, `shop_template_side` 3822 /
//! `shop_template_side:inv` 3823 for Sell) and their fixed op slots; the
//! compact snapshot seam owns where those containers currently are and what
//! they hold. JavaScript starts one machine per call and awaits the frozen
//! result — the 10/5/1 batching, the packet-per-tick bound, the waits and
//! the held-count settlement policy stay here. A queued click is not a
//! transfer: a batch is only counted once the posted container counts moved.

use crate::machine::{Begin, Cx, Family, Step};
use crate::observed::{self, ItemRow, Scene, Text};
use crate::shim::InteractReq;
use serde::Deserialize;
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Kind {
    Open,
    Buy,
    Sell,
    Close,
}

impl Kind {
    /// The frozen settle value: `open` a boolean, `buy`/`sell` the observed
    /// count, `close` void.
    fn value(self, result: bool, quantity: i32) -> Value {
        match self {
            Self::Open => json!(result),
            Self::Close => Value::Null,
            Self::Buy | Self::Sell => json!(quantity),
        }
    }
}

/// Transfer phase of the current operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Waiting for the posted open/close boundary.
    WaitBoundary,
    /// A batch of Buy/Sell ops is outstanding.
    WaitBatch,
    /// Settle one poll after a batch: the frozen `delayTicks(1)` that lets the
    /// batch's own server tick land before the recount.
    SettleTick,
}

#[derive(Deserialize)]
pub(crate) struct ShopArgs {
    kind: Kind,
    #[serde(default)]
    name: String,
    /// A positive count or `"all"`; anything else is an invalid request.
    #[serde(default)]
    qty: Value,
}

/// One frozen `Shop.open`/`buy`/`sell`/`close` call.
pub(crate) struct Shop {
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
    /// Remaining `Shop.open` attempts.
    attempts_left: u32,
}

impl Family for Shop {
    const NAME: &'static str = "shop";
    /// A new shop call replaces the one in flight.
    const EXCLUSIVE: bool = true;
    type Args = ShopArgs;
    type Output = Value;

    fn begin(args: ShopArgs, cx: &mut Cx<'_>) -> Begin<Self> {
        let kind = args.kind;
        let name = args.name.trim().to_string();
        let obs = observed::with(|scene| NativeObservation::from_scene(scene, kind == Kind::Open));
        let probe = obs.probe();
        // Not in game: nothing is sent and the call settles empty.
        let refused = Begin::Done(kind.value(false, 0));
        if !probe.ingame {
            return refused;
        }
        let mut shop = Self::new(kind);
        match kind {
            Kind::Open => {
                // The frozen `Shop.open` returns false when no posted NPC
                // offers that name with a Trade op: that is an absent world
                // object, not a missing host capability.
                let Some((npc_name, action)) = trade_target(probe.npcs, &name) else {
                    return refused;
                };
                if probe.shop_open {
                    // Already open: success with no Trade press.
                    return Begin::Done(kind.value(true, 0));
                }
                shop.name = npc_name;
                shop.npc_action = action;
                shop.attempts_left = OPEN_ATTEMPTS;
                shop.phase = Phase::WaitBoundary;
                shop.press_trade(cx);
                Begin::Run(shop)
            }
            Kind::Close => {
                if !probe.shop_open {
                    return Begin::Done(kind.value(true, 0));
                }
                shop.phase = Phase::WaitBoundary;
                cx.clock().arm(CLOSE_WAIT_MS);
                cx.emit(InteractReq::CloseModal);
                Begin::Run(shop)
            }
            Kind::Buy | Kind::Sell => {
                if !probe.shop_open || name.is_empty() {
                    return refused;
                }
                let container = match kind {
                    Kind::Buy if !probe.has_stock => {
                        return Begin::Refuse("missing shop stock".into());
                    }
                    Kind::Buy => probe.stock,
                    _ => match probe.player {
                        Some(player) => player,
                        None => return Begin::Refuse("missing shop player pack".into()),
                    },
                };
                let Some(row) = find_row(container, &name).cloned() else {
                    return refused;
                };
                // The frozen `Shop.buy` breaks out on a stock row already at zero.
                if kind == Kind::Buy && row.count <= 0 {
                    return refused;
                }
                let requested = match &args.qty {
                    Value::String(all) if all == "all" => row.count,
                    // A JS number beyond int32 reaches Rust as a float.
                    qty => match qty
                        .as_i64()
                        .or_else(|| qty.as_f64().filter(|n| n.fract() == 0.0).map(|n| n as i64))
                    {
                        Some(qty) if qty >= 1 => qty as i32,
                        _ => return refused,
                    },
                };
                if requested < 1 {
                    return refused;
                }
                shop.name = row.name.to_string();
                shop.requested = requested;
                shop.held_now = count_of(probe.inv, &row.name);
                match shop.send_batch(&row, cx) {
                    Step::Done(value) => Begin::Done(value),
                    _ => Begin::Run(shop),
                }
            }
        }
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
        let obs = observed::with(|scene| NativeObservation::from_scene(scene, false));
        let probe = obs.probe();
        if !probe.ingame {
            return Step::Done(self.kind.value(false, 0));
        }
        match self.kind {
            Kind::Open => self.open_step(&probe, cx),
            Kind::Close => self.close_step(&probe, cx),
            Kind::Buy | Kind::Sell => self.transfer_step(&probe, cx),
        }
    }
}

impl Shop {
    fn new(kind: Kind) -> Self {
        Self {
            phase: Phase::WaitBoundary,
            kind,
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

    fn done(&self, result: bool) -> Step<Value> {
        Step::Done(self.kind.value(result, self.transferred))
    }

    /// Press the NPC's Trade op once and open this attempt's window.
    fn press_trade(&mut self, cx: &mut Cx<'_>) {
        self.pressed = true;
        cx.clock().arm(OPEN_WAIT_MS);
        cx.emit(InteractReq::Npc {
            name: self.name.clone(),
            action: self.npc_action.clone(),
            index: None,
        });
    }

    /// Emit the next batch of Buy/Sell ops for the outstanding remainder and
    /// arm the held-count settlement window.
    fn send_batch(&mut self, row: &Row, cx: &mut Cx<'_>) -> Step<Value> {
        let chunks = plan(self.requested - self.transferred);
        if chunks.is_empty() {
            return self.done(self.transferred >= self.requested);
        }
        let kind = if self.kind == Kind::Buy {
            "buy"
        } else {
            "sell"
        };
        for chunk in chunks {
            cx.emit(InteractReq::ShopButton {
                kind: kind.into(),
                name: row.name.to_string(),
                id: row.id,
                slot: row.slot,
                component: row.component,
                chunk,
            });
        }
        self.batch_delta = 0;
        self.batch_baseline = self.held_now;
        self.phase = Phase::WaitBatch;
        cx.clock().arm(SETTLE_MS);
        Step::Wait
    }

    fn open_step(&mut self, probe: &Probe<'_>, cx: &mut Cx<'_>) -> Step<Value> {
        if probe.shop_open {
            return self.done(true);
        }
        // The frozen `Shop.open` presses Trade once per attempt and only
        // re-presses after that attempt's own 3000 ms window expired.
        if !self.pressed {
            self.press_trade(cx);
            return Step::Wait;
        }
        if !cx.clock().bound_reached() {
            return Step::Wait;
        }
        self.attempts_left = self.attempts_left.saturating_sub(1);
        if self.attempts_left == 0 {
            return self.done(false);
        }
        self.press_trade(cx);
        Step::Wait
    }

    fn close_step(&mut self, probe: &Probe<'_>, cx: &mut Cx<'_>) -> Step<Value> {
        if !probe.shop_open || cx.clock().bound_reached() {
            return self.done(!probe.shop_open);
        }
        Step::Wait
    }

    fn transfer_step(&mut self, probe: &Probe<'_>, cx: &mut Cx<'_>) -> Step<Value> {
        if !probe.shop_open {
            return self.done(false);
        }
        let container = match self.kind {
            Kind::Buy if !probe.has_stock => return self.done(false),
            Kind::Buy => probe.stock,
            // The shop's player pack is our Sell identity: losing it stops
            // the transfer instead of falling back to the backpack.
            _ => match probe.player {
                Some(player) => player,
                None => return self.done(false),
            },
        };
        let Some(row) = find_row(container, &self.name).cloned() else {
            return self.done(self.transferred >= self.requested);
        };
        // The frozen `Shop.buy` breaks out on a stock row already at zero.
        if self.kind == Kind::Buy && row.count <= 0 {
            return self.done(self.transferred >= self.requested);
        }
        match self.phase {
            Phase::WaitBatch => {
                let delta = self.batch_delta(probe);
                if delta > 0 {
                    self.batch_delta = delta;
                    self.phase = Phase::SettleTick;
                } else if cx.clock().bound_reached() {
                    // The window closed with no movement; the batch's own
                    // server tick may still land, so settle exactly one more
                    // tick before declaring the transfer stalled.
                    self.phase = Phase::SettleTick;
                }
                Step::Wait
            }
            Phase::SettleTick => {
                let delta = self.batch_delta(probe).max(self.batch_delta);
                self.transferred += delta;
                self.held_now = count_of(probe.inv, &self.name);
                if self.transferred >= self.requested {
                    return self.done(true);
                }
                if delta == 0 {
                    // Nothing moved across the settle window and its one tick.
                    return self.done(false);
                }
                self.send_batch(&row, cx)
            }
            Phase::WaitBoundary => self.done(false),
        }
    }

    /// The items this batch moved, read from the posted backpack counts: Buy
    /// adds to the pack, Sell takes from it.
    fn batch_delta(&self, probe: &Probe<'_>) -> i32 {
        let held = count_of(probe.inv, &self.name);
        match self.kind {
            Kind::Buy => (held - self.batch_baseline).max(0),
            _ => (self.batch_baseline - held).max(0),
        }
    }
}

pub fn configure(data: Option<Arc<api::game_data::SelectedGameData>>) {
    GAME_DATA.with(|slot| *slot.borrow_mut() = data);
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
    use crate::task_clock::InstantTaskClock;
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

    fn shop(kind: Kind, name: &str, requested: i32, phase: Phase) -> Shop {
        let mut shop = Shop::new(kind);
        shop.name = name.into();
        shop.requested = requested;
        shop.phase = phase;
        shop
    }

    fn probe<'a>(stock: &'a [Row], player: Option<&'a [Row]>, inv: &'a [Row]) -> Probe<'a> {
        Probe {
            ingame: true,
            shop_open: true,
            has_stock: true,
            stock,
            player,
            inv,
            npcs: &[],
        }
    }

    fn armed(window: u64) -> InstantTaskClock {
        let mut clock = InstantTaskClock::new();
        clock.arm(window);
        clock
    }

    fn expire(clock: &mut InstantTaskClock) {
        clock.deadline = Some(Instant::now() - Duration::from_millis(1));
    }

    /// One step: `None` while it waits, else the settle value; plus the ops.
    fn step(
        shop: &mut Shop,
        clock: &mut InstantTaskClock,
        probe: &Probe<'_>,
    ) -> (Option<Value>, Vec<InteractReq>) {
        let mut ops = Vec::new();
        let mut cx = Cx::test(&mut ops, clock, None);
        let step = match shop.kind {
            Kind::Open => shop.open_step(probe, &mut cx),
            Kind::Close => shop.close_step(probe, &mut cx),
            Kind::Buy | Kind::Sell => shop.transfer_step(probe, &mut cx),
        };
        let out = match step {
            Step::Wait => None,
            Step::Done(value) => Some(value),
            _ => panic!("the shop neither calls back nor fails"),
        };
        (out, ops)
    }

    #[test]
    fn a_queued_batch_is_not_a_transfer_until_the_counts_move() {
        let mut rt = shop(Kind::Buy, "Feather", 10, Phase::WaitBatch);
        rt.batch_baseline = 4;
        let mut clock = armed(SETTLE_MS);
        let inv = [row("Feather", 4, 1)];
        let stock = [row("Feather", 20, 3)];
        let probe = probe(&stock, None, &inv);
        // Nothing moved yet: the batch still waits inside its window.
        assert_eq!(step(&mut rt, &mut clock, &probe), (None, vec![]));
        assert_eq!(rt.transferred, 0);
        // Past the window the batch settles exactly one more tick (the tick
        // the frozen loop waits for its batch to land in) before it counts.
        expire(&mut clock);
        assert_eq!(step(&mut rt, &mut clock, &probe), (None, vec![]));
        // Only a recount that is still empty stalls the transfer at 0.
        assert_eq!(step(&mut rt, &mut clock, &probe), (Some(json!(0)), vec![]));
    }

    #[test]
    fn open_presses_trade_once_per_attempt_and_bounds_at_three() {
        let mut rt = shop(Kind::Open, "Shop keeper", 0, Phase::WaitBoundary);
        rt.npc_action = "Trade".into();
        rt.attempts_left = OPEN_ATTEMPTS;
        rt.pressed = true;
        let mut clock = armed(OPEN_WAIT_MS);
        let closed = Probe {
            shop_open: false,
            has_stock: false,
            ..probe(&[], None, &[])
        };
        let trade = InteractReq::Npc {
            name: "Shop keeper".into(),
            action: "Trade".into(),
            index: None,
        };
        // Inside the attempt window the frozen loop only waits: no re-press.
        assert_eq!(step(&mut rt, &mut clock, &closed), (None, vec![]));
        // Each expired window re-presses once, for the frozen three attempts.
        for attempt in 1..OPEN_ATTEMPTS {
            expire(&mut clock);
            assert_eq!(
                step(&mut rt, &mut clock, &closed),
                (None, vec![trade.clone()]),
                "attempt {attempt} re-presses Trade"
            );
            assert_eq!(
                step(&mut rt, &mut clock, &closed),
                (None, vec![]),
                "one press per window"
            );
        }
        // The last window closes the call as a timeout, not another press.
        expire(&mut clock);
        assert_eq!(
            step(&mut rt, &mut clock, &closed),
            (Some(json!(false)), vec![])
        );
    }

    #[test]
    fn open_reports_success_once_the_posted_shop_is_up() {
        let mut rt = shop(Kind::Open, "Shop keeper", 0, Phase::WaitBoundary);
        rt.attempts_left = OPEN_ATTEMPTS;
        rt.pressed = true;
        let mut clock = armed(OPEN_WAIT_MS);
        let open = probe(&[], None, &[]);
        assert_eq!(
            step(&mut rt, &mut clock, &open),
            (Some(json!(true)), vec![])
        );
    }

    #[test]
    fn close_settles_void_on_the_closed_boundary() {
        let mut rt = shop(Kind::Close, "", 0, Phase::WaitBoundary);
        let mut clock = armed(CLOSE_WAIT_MS);
        let open = probe(&[], None, &[]);
        assert_eq!(step(&mut rt, &mut clock, &open), (None, vec![]));
        let closed = Probe {
            shop_open: false,
            ..open
        };
        assert_eq!(
            step(&mut rt, &mut clock, &closed),
            (Some(Value::Null), vec![])
        );
    }

    #[test]
    fn a_depleted_stock_row_ends_buy_with_the_partial_count() {
        let mut rt = shop(Kind::Buy, "Feather", 25, Phase::SettleTick);
        rt.transferred = 10;
        let mut clock = InstantTaskClock::new();
        let empty = [row("Feather", 0, 3)];
        let inv = [row("Feather", 12, 1)];
        assert_eq!(
            step(&mut rt, &mut clock, &probe(&empty, None, &inv)),
            (Some(json!(10)), vec![]),
            "partial progress is reported"
        );
    }

    #[test]
    fn a_served_batch_settles_one_tick_then_continues_the_remainder() {
        let mut rt = shop(Kind::Buy, "Feather", 25, Phase::WaitBatch);
        rt.transferred = 10;
        rt.batch_baseline = 10;
        let mut clock = armed(SETTLE_MS);
        let stock = [row("Feather", 90, 3)];
        let inv = [row("Feather", 20, 1)];
        let probe = probe(&stock, None, &inv);
        // The batch moved 10 of the 10 requested: settle one tick, no count yet.
        assert_eq!(step(&mut rt, &mut clock, &probe), (None, vec![]));
        assert_eq!(rt.transferred, 10, "settlement is not added yet");

        // One tick later the delta is added and the remainder is batch-planned.
        let (out, ops) = step(&mut rt, &mut clock, &probe);
        assert_eq!(out, None);
        assert_eq!(rt.transferred, 20);
        assert_eq!(
            ops,
            vec![InteractReq::ShopButton {
                kind: "buy".into(),
                name: "Feather".into(),
                id: 221,
                slot: 3,
                component: 3900,
                chunk: 5,
            }],
            "remaining 5 is the frozen 5 chunk on the exact stock row"
        );
    }

    #[test]
    fn buy_settles_total_across_separate_matching_inventory_slots() {
        let mut rt = shop(Kind::Buy, "Vial", 15, Phase::WaitBatch);
        let mut clock = armed(SETTLE_MS);
        let stock = [row("Vial", 20, 3), row("Feather", 99, 4)];
        let inv = [
            row("Vial", 10, 1),
            row("Coins", 1900, 2),
            row("vIaL", 5, 7),
            row("Feather", 99, 8),
        ];
        let probe = probe(&stock, None, &inv);
        assert_eq!(step(&mut rt, &mut clock, &probe), (None, vec![]));
        assert_eq!(step(&mut rt, &mut clock, &probe), (Some(json!(15)), vec![]));
    }

    #[test]
    fn sell_plans_against_the_shop_player_pack_and_counts_inventory_losses() {
        let mut rt = shop(Kind::Sell, "Vial", 10, Phase::SettleTick);
        rt.batch_baseline = 12;
        rt.batch_delta = 10;
        let mut clock = InstantTaskClock::new();
        let player = [row("Vial", 2, 4)];
        let inv = [row("Vial", 2, 8)];
        assert_eq!(
            step(&mut rt, &mut clock, &probe(&[], Some(&player), &inv)),
            (Some(json!(10)), vec![])
        );
    }

    #[test]
    fn sell_settles_total_across_separate_matching_inventory_slots() {
        let mut rt = shop(Kind::Sell, "Vial", 10, Phase::WaitBatch);
        rt.batch_baseline = 20;
        let mut clock = armed(SETTLE_MS);
        let player = [row("Vial", 7, 4), row("Coins", 2000, 5), row("vIaL", 3, 9)];
        let inv = [
            row("Vial", 4, 8),
            row("Feather", 99, 10),
            row("vIaL", 6, 11),
        ];
        let probe = probe(&[], Some(&player), &inv);
        assert_eq!(step(&mut rt, &mut clock, &probe), (None, vec![]));
        assert_eq!(step(&mut rt, &mut clock, &probe), (Some(json!(10)), vec![]));
    }

    #[test]
    fn losing_the_shop_player_pack_stops_sell_without_touching_the_backpack() {
        let mut rt = shop(Kind::Sell, "Vial", 5, Phase::WaitBatch);
        let mut clock = armed(SETTLE_MS);
        let inv = [row("Vial", 9, 8)];
        assert_eq!(
            step(&mut rt, &mut clock, &probe(&[], None, &inv)),
            (Some(json!(0)), vec![])
        );
    }
}
