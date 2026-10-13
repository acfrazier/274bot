//! Rust-owned observed shop open/buy/sell/sellAll/close sequencing, the `shop`
//! [`crate::machine`] family.
//!
//! The selected caches own the shop interface identities (`shop_template`
//! 3824 / `shop_template:inv` 3900 for Buy, `shop_template_side` 3822 /
//! `shop_template_side:inv` 3823 for Sell) and their fixed op slots; the
//! compact snapshot seam owns where those containers currently are and what
//! they hold. JavaScript starts one machine per call and awaits the frozen
//! result — the 10/5/1 batching, the packet-per-tick bound, the waits and
//! the held-count settlement policy stay here. `Shop.sell` and `Shop.sellAll`
//! call the optional `pick` hook over same-name shop player rows in order
//! before each batch (for sellAll, each click). A queued click is not
//! a transfer: a batch is only counted once the posted container counts moved.

use crate::machine::{Begin, Call, Cx, Family, Reply, Step};
use crate::observed::{self, ItemRow, Scene, Text};
use crate::reach_route;
use crate::shim::InteractReq;
use crate::shop_open::{self, Decision, Facts, Keeper, OpenCore, Session};
use crate::walk::{Resilient, HOP_ATTEMPTS};
use api::WorldTile;
use serde::Deserialize;
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Frozen `Shop.close`: wait for `!isOpen` this long.
pub const CLOSE_WAIT_MS: u64 = 3_000;
/// Held-count settlement window before a batch counts as stalled.
pub const SETTLE_MS: u64 = 3_000;
/// Frozen batch bound: at most this many user-event packets in one tick.
pub const MAX_PACKETS_PER_TICK: usize = 5;
/// Frozen `src/bot/api/shop/Shop.ts:151-176,203-205`: sale attempts stop
/// after 40 clicks even if the shop keeps accepting stock.
const SELL_ALL_CLICKS: usize = 40;

thread_local! {
    static GAME_DATA: RefCell<Option<Arc<api::game_data::SelectedGameData>>> =
        const { RefCell::new(None) };
    /// `(scene_epoch, tick)` of the latest `OpNpc` Trade emitted by `Shop.open`.
    /// Every `OpNpc` closes the active modal on the server (`OpNpcHandler.ts:47`
    /// -> `Player.ts:970-973` `clearPendingAction` -> `closeModal`), so
    /// `Shop.open` must not settle and `Shop.buy` must not emit `ShopButton`
    /// (`InvButtonHandler.ts:23-25`) on a tick that already emitted `OpNpc`.
    static LAST_TRADE_TICK: Cell<Option<(u64, u64)>> = const { Cell::new(None) };
}

fn trade_emitted_on(epoch: u64, tick: u64) -> bool {
    tick != 0 && LAST_TRADE_TICK.with(|cell| cell.get() == Some((epoch, tick)))
}

/// Debug-only `Shop.open` / batch timeline with game ticks, so a live receipt
/// shows the Trade, its deadline, the open settle and the first button.
fn trace(event: &str, tick: Option<u64>, detail: std::fmt::Arguments<'_>) {
    if !api::hostlog::debug_enabled() {
        return;
    }
    api::host_log!(
        api::hostlog::Category::ScriptTrace,
        api::hostlog::Level::Debug,
        "shop {event} tick={tick:?} {detail}"
    );
}

/// Bound of the nav walk to an unreachable keeper: the frozen
/// `walkResilient` baked timeout and hop attempts the other approach walks
/// use ([`crate::walk::BAKED_TIMEOUT_MS`], [`crate::walk::HOP_ATTEMPTS`]).
const KEEPER_WALK_MS: u64 = crate::walk::BAKED_TIMEOUT_MS;

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
    /// The posted game tick, `None` on the logout post.
    tick: Option<u64>,
    /// The session these facts belong to.
    session: Session,
    /// Highest chat line seq this login session (the Trade's mark).
    chat_seq: i32,
    /// Highest seq of an "I can't reach that!" line this login session.
    cant_reach_seq: i32,
    ingame: bool,
    shop_open: bool,
    has_stock: bool,
    stock: Vec<Row>,
    /// `None` = the shop player pack was not decoded by the last rebuild.
    player: Option<Vec<Row>>,
    inv: Vec<Row>,
    /// The keeper name `Shop.open` asked for (empty for other calls).
    keeper: String,
}

impl NativeObservation {
    /// A logout forgets the session: only pages posted since login count.
    fn from_scene(scene: &Scene, keeper: &str) -> Self {
        let session = scene.since_login();
        let lines = session.chat_lines().map_or(&[][..], Vec::as_slice);
        let seq_of = |cant_reach: bool| {
            lines
                .iter()
                .filter(|line| !cant_reach || shop_open::is_cant_reach(&line.text))
                .map(|line| line.seq)
                .max()
                .unwrap_or(0)
        };
        Self {
            tick: scene.session_tick(),
            session: Session {
                epoch: scene.epoch(),
                login: scene.login_mark(),
            },
            chat_seq: seq_of(false),
            cant_reach_seq: seq_of(true),
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
            keeper: keeper.to_string(),
        }
    }

    fn probe(&self) -> Probe<'_> {
        Probe {
            tick: self.tick,
            session: self.session,
            chat_seq: self.chat_seq,
            cant_reach_seq: self.cant_reach_seq,
            ingame: self.ingame,
            shop_open: self.shop_open,
            has_stock: self.has_stock,
            stock: &self.stock,
            player: self.player.as_deref(),
            inv: &self.inv,
            keeper: KeeperFact::Scene(&self.keeper),
        }
    }
}

/// The keeper `Shop.open` decides from. The live lookup runs only when a
/// decision needs it (a Trade or a walk), not on every waiting tick.
#[derive(Clone, Copy)]
enum KeeperFact<'a> {
    /// Look the named keeper up in the live scene.
    Scene(&'a str),
    /// A fixed answer.
    #[cfg(test)]
    Given(Option<&'a Keeper>),
}

impl KeeperFact<'_> {
    fn resolve(self) -> Option<Keeper> {
        match self {
            Self::Scene(name) => observed::with(|scene| {
                let session = scene.since_login();
                let here = session.here().map(|t| WorldTile {
                    x: t.x,
                    z: t.z,
                    level: t.level,
                });
                let rows = session.npcs().map_or(&[][..], Vec::as_slice);
                crate::load::reach_query::with_view(|view| {
                    let live = here.and_then(|here| {
                        reach_route::live(Some(view), Some(here)).map(|view| (view, here))
                    });
                    choose_keeper(rows, name, live)
                })
            }),
            #[cfg(test)]
            Self::Given(keeper) => keeper.cloned(),
        }
    }
}

/// The posted NPC rows named `name`, offered to the shared chooser
/// ([`shop_open::choose_keeper`]). Frozen `Shop.open` presses the `Trade`
/// op. The footprint is the network tile and size, the tile the client's
/// `tryMove` routes to (`client.rs:3988-3998`).
fn choose_keeper(
    rows: &[observed::EntityRow],
    name: &str,
    view: Option<(&api::query::ReachQueryView, WorldTile)>,
) -> Option<Keeper> {
    let name = name.trim();
    let candidates = rows
        .iter()
        .filter(|row| {
            row.name
                .as_deref()
                .is_some_and(|n| n.trim().eq_ignore_ascii_case(name))
        })
        .map(|row| shop_open::Candidate {
            name: row.name.as_deref().unwrap_or_default(),
            action: row
                .actions
                .iter()
                .find(|op| op.eq_ignore_ascii_case("trade"))
                .map(|op| &**op),
            index: row.index,
            tile: WorldTile {
                x: if row.size >= 1 { row.nx } else { row.x },
                z: if row.size >= 1 { row.nz } else { row.z },
                level: row.level,
            },
            size: row.size,
            distance: row.distance,
        });
    shop_open::choose_keeper(candidates, view)
}

/// What the caller asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Kind {
    Open,
    Buy,
    Sell,
    SellAll,
    Close,
}

impl Kind {
    /// The frozen settle value: `open` a boolean, transfers the observed
    /// count, `close` void.
    fn value(self, result: bool, quantity: i32) -> Value {
        match self {
            Self::Open => json!(result),
            Self::Close => Value::Null,
            Self::Buy | Self::Sell | Self::SellAll => json!(quantity),
        }
    }
}

/// Transfer phase of the current operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Waiting for the posted open/close boundary.
    WaitBoundary,
    /// Calling `pick` on each same-name candidate in order.
    Pick,
    /// A batch of Buy/Sell ops is outstanding.
    WaitBatch,
    /// Waiting for a later server cycle before dispatching the next batch.
    ReadyBatch,
}

/// The caller's `Shop.sell` `pick` hook.
const PICK: usize = 0;

#[derive(Deserialize)]
pub(crate) struct ShopArgs {
    kind: Kind,
    #[serde(default)]
    name: String,
    /// A positive count or `"all"`; anything else is an invalid request.
    #[serde(default)]
    qty: Value,
}

/// One frozen `Shop.open`/`buy`/`sell`/`sellAll`/`close` call.
pub(crate) struct Shop {
    phase: Phase,
    kind: Kind,
    /// The selected row's name (Shop.open: the keeper's posted name).
    name: String,
    /// Requested amount (`Buy`/`Sell`/`SellAll`; 0 for open/close).
    requested: i32,
    /// `qty: "all"` or `SellAll` under `pick`: resolve after choosing a row.
    all: bool,
    /// `Shop.sell` or `Shop.sellAll` was given a `pick`.
    pick: bool,
    /// Number of SellAll button clicks emitted so far (not transferred items).
    clicks: usize,
    /// Same-name shop player rows `pick` has not answered yet, in order.
    candidates: Vec<Row>,
    /// Amount observed to have moved so far.
    transferred: i32,
    /// Largest observed amount moved by the outstanding batch.
    batch_delta: i32,
    /// The observed held count when the outstanding batch was sent.
    batch_baseline: i32,
    /// Last observed game tick in which this row emitted a shop batch.
    last_dispatch_tick: Option<u64>,
    /// The observed held count now (kept for the next batch baseline).
    held_now: i32,
    /// `Shop.open`'s shared core state.
    open: OpenCore,
    /// The nav walk to an unreachable keeper, while one is out.
    walk: Option<Box<Resilient>>,
}

impl Family for Shop {
    const NAME: &'static str = "shop";
    /// A new shop call replaces the one in flight.
    const EXCLUSIVE: bool = true;
    const SNAPSHOT_SENSITIVE: bool = true;
    const CALLBACKS: &'static [&'static str] = &["pick"];
    /// `pick` runs in the caller's tick, like the frozen `find(pick)`.
    const KICK_ON_START: bool = true;
    /// Frozen `matches.find(pick)` does not await: a promise is truthy.
    const SYNC_HOOKS: &'static [usize] = &[PICK];
    type Args = ShopArgs;
    type Output = Value;

    fn begin(args: ShopArgs, cx: &mut Cx<'_>) -> Begin<Self> {
        let kind = args.kind;
        let name = args.name.trim().to_string();
        let obs = observed::with(|scene| NativeObservation::from_scene(scene, &name));
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
                let Some(keeper) = probe.keeper.resolve() else {
                    return refused;
                };
                shop.name = keeper.name;
                shop.phase = Phase::WaitBoundary;
                let facts = probe.facts();
                if probe.shop_open {
                    // The modal and its stock container can be published on
                    // adjacent snapshots. Keep this one await open until the
                    // page is usable so an immediate Shop.buy cannot observe
                    // a transient empty row set.
                    if shop.open.ready(&facts) {
                        return Begin::Done(kind.value(true, 0));
                    }
                    shop.open = OpenCore::awaiting_page(&facts);
                    return Begin::Run(shop);
                }
                let decision = shop.open.tick(&facts, || probe.keeper.resolve());
                match shop.act(decision, &probe, cx) {
                    Step::Done(value) => Begin::Done(value),
                    _ => Begin::Run(shop),
                }
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
            Kind::Buy | Kind::Sell | Kind::SellAll => {
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
                if matches!(kind, Kind::Sell | Kind::SellAll) && cx.has(PICK) {
                    let candidates: Vec<Row> = container
                        .iter()
                        .filter(|row| row.name.eq_ignore_ascii_case(&name))
                        .cloned()
                        .collect();
                    let Some(first) = candidates.first() else {
                        return refused;
                    };
                    shop.all = kind == Kind::SellAll || args.qty == Value::String("all".into());
                    shop.requested = if shop.all {
                        0
                    } else {
                        match requested(&args.qty) {
                            Some(qty) => qty,
                            None => return refused,
                        }
                    };
                    shop.name = first.name.to_string();
                    shop.pick = true;
                    shop.candidates = candidates;
                    shop.phase = Phase::Pick;
                    return Begin::Run(shop);
                }
                let Some(row) = find_row(container, &name).cloned() else {
                    return refused;
                };
                // The frozen `Shop.buy` breaks out on a stock row already at zero.
                if kind == Kind::Buy && row.count <= 0 {
                    return refused;
                }
                let requested = match &args.qty {
                    _ if kind == Kind::SellAll => i32::MAX,
                    Value::String(all) if all == "all" => row.count,
                    qty => match requested(qty) {
                        Some(qty) => qty,
                        None => return refused,
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
        let reply = cx.reply();
        if let Some(Reply::Threw(thrown)) = reply {
            return Step::Fail(thrown);
        }
        let obs = observed::with(|scene| NativeObservation::from_scene(scene, &self.name));
        let probe = obs.probe();
        if !probe.ingame {
            return Step::Done(self.kind.value(false, 0));
        }
        if self.phase == Phase::Pick {
            return self.pick_step(&probe, reply, cx);
        }
        match self.kind {
            Kind::Open => self.open_step(&probe, cx),
            Kind::Close => self.close_step(&probe, cx),
            Kind::Buy | Kind::Sell | Kind::SellAll => self.transfer_step(&probe, cx),
        }
    }

    fn snapshot_step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
        let reply = cx.reply();
        if let Some(Reply::Threw(thrown)) = reply {
            return Step::Fail(thrown);
        }
        let obs = observed::with(|scene| NativeObservation::from_scene(scene, ""));
        let probe = obs.probe();
        if !probe.ingame {
            return Step::Done(self.kind.value(false, 0));
        }
        if self.phase == Phase::Pick {
            return self.pick_step(&probe, reply, cx);
        }
        match self.kind {
            // A snapshot may confirm the page. Trades and attempt expiry are
            // owned by real ticks, so a snapshot never sends or fails.
            Kind::Open if self.open.ready(&probe.facts()) => self.open_settled(&probe),
            Kind::Open => Step::Wait,
            Kind::Close => self.close_step(&probe, cx),
            Kind::Buy | Kind::Sell | Kind::SellAll => self.transfer_step(&probe, cx),
        }
    }

    /// A nav walk to an unreachable keeper is stopped when the call ends
    /// early, like the other approach walks ([`Resilient::release`]).
    fn release(&self) -> Option<InteractReq> {
        self.walk.as_ref().and_then(|walk| walk.release())
    }
}

impl Shop {
    fn new(kind: Kind) -> Self {
        Self {
            phase: Phase::WaitBoundary,
            kind,
            name: String::new(),
            requested: 0,
            all: false,
            pick: false,
            candidates: Vec::new(),
            transferred: 0,
            batch_delta: 0,
            last_dispatch_tick: None,
            batch_baseline: 0,
            held_now: 0,
            clicks: 0,
            open: OpenCore::default(),
            walk: None,
        }
    }

    /// Ask `pick` about the next candidate; the first truthy answer is the
    /// row this batch sells from. No row accepted ends the sale.
    fn pick_step(
        &mut self,
        probe: &Probe<'_>,
        reply: Option<Reply>,
        cx: &mut Cx<'_>,
    ) -> Step<Value> {
        if let Some(Reply::Value(picked)) = reply {
            if crate::trade::truthy(&picked) {
                let row = self.candidates.remove(0);
                self.candidates.clear();
                if self.all {
                    self.all = false;
                    self.requested = if self.kind == Kind::SellAll {
                        i32::MAX
                    } else {
                        row.count
                    };
                }
                self.held_now = count_of(probe.inv, &self.name);
                return self.send_batch(&row, cx);
            }
            self.candidates.remove(0);
        }
        if !probe.shop_open {
            return self.done(false);
        }
        let Some(row) = self.candidates.first() else {
            return self.done(false);
        };
        Step::Call(Call {
            hook: PICK,
            args: vec![json!({
                "id": row.id,
                "name": row.name.as_ref(),
                "count": row.count,
                "slot": row.slot,
                "comId": row.component,
            })],
        })
    }

    fn done(&self, result: bool) -> Step<Value> {
        Step::Done(self.kind.value(result, self.transferred))
    }

    /// `Shop.open` ends false with `reason` in the slot log.
    fn open_failed(&self, tick: Option<u64>, reason: std::fmt::Arguments<'_>) -> Step<Value> {
        api::host_log!(
            api::hostlog::Category::ScriptLifecycle,
            api::hostlog::Level::Warn,
            "Shop.open('{}') failed at tick {tick:?}: {reason}",
            self.name
        );
        self.done(false)
    }

    /// Send the keeper's Trade and start its attempt in the core. The core
    /// never asks for one before the session's first tick or on a tick that
    /// already sent a Trade (`Facts::trade_taken`).
    fn press_trade(&mut self, probe: &Probe<'_>, keeper: &Keeper, cx: &mut Cx<'_>) {
        let Some(tick) = probe.tick else {
            return;
        };
        let epoch = observed::with(Scene::epoch);
        LAST_TRADE_TICK.with(|cell| cell.set(Some((epoch, tick))));
        self.open.traded(&probe.facts(), keeper);
        trace(
            "trade",
            Some(tick),
            format_args!(
                "npc={} index={} route={:?} deadline={:?}",
                keeper.name,
                keeper.index,
                keeper.route,
                self.open.deadline()
            ),
        );
        cx.emit(InteractReq::Npc {
            name: keeper.name.clone(),
            action: keeper.action.clone(),
            index: Some(keeper.index),
        });
    }

    /// This tick already sent a shop batch or a Trade: a button now would
    /// race the Trade's modal close (`OpNpcHandler.ts:47`).
    fn tick_spent(&self) -> bool {
        let (epoch, tick) = observed::with(|scene| (scene.epoch(), scene.tick().unwrap_or(0)));
        self.last_dispatch_tick == Some(tick) || trade_emitted_on(epoch, tick)
    }

    /// Emit the next batch of Buy/Sell ops for the outstanding remainder and
    /// arm the held-count settlement window.
    fn send_batch(&mut self, row: &Row, cx: &mut Cx<'_>) -> Step<Value> {
        if self.tick_spent() {
            self.phase = Phase::ReadyBatch;
            return Step::Wait;
        }
        let tick = observed::with(|scene| scene.tick().unwrap_or(0));
        if self.kind == Kind::SellAll && self.clicks >= SELL_ALL_CLICKS {
            return self.done(false);
        }
        let chunks = if self.kind == Kind::SellAll {
            // Frozen `Shop.ts:151-176`: one Sell 10 per settled click;
            // re-read the player pack and re-run `pick` before the next one.
            self.clicks += 1;
            None
        } else {
            Some(plan(self.requested - self.transferred))
        };
        if chunks.as_ref().is_some_and(Vec::is_empty) {
            return self.done(self.transferred >= self.requested);
        }
        let kind = if self.kind == Kind::Buy {
            "buy"
        } else {
            "sell"
        };
        self.last_dispatch_tick = Some(tick);
        trace(
            "button",
            Some(tick),
            format_args!("kind={kind} item={} chunks={chunks:?}", row.name),
        );
        for &chunk in chunks.as_deref().unwrap_or(&[10]) {
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

    /// The core settles only on a usable page ([`OpenCore::ready`]). Modal
    /// and stock are separate packets (`IF_OPENMAIN_SIDE` in
    /// `NetworkPlayer.ts:155-182`, `UPDATE_INV_FULL` from `updateInvs`), so
    /// the modal alone is not usable.
    fn open_settled(&self, probe: &Probe<'_>) -> Step<Value> {
        trace("open", probe.tick, format_args!("settled=true"));
        self.done(true)
    }

    /// One real tick of `Shop.open`: drive the nav walk if one is out, else
    /// let the shared core decide ([`OpenCore::tick`]).
    fn open_step(&mut self, probe: &Probe<'_>, cx: &mut Cx<'_>) -> Step<Value> {
        if let Some(walk) = self.walk.as_mut() {
            let ended = walk.step(cx);
            while let Some(line) = walk.pop_log() {
                trace("walk", probe.tick, format_args!("{line}"));
            }
            let Some(arrived) = ended else {
                return Step::Wait;
            };
            self.walk = None;
            let decision = self.open.walked(arrived, || probe.keeper.resolve());
            return self.act(decision, probe, cx);
        }
        let decision = self.open.tick(&probe.facts(), || probe.keeper.resolve());
        self.act(decision, probe, cx)
    }

    /// Carry out one core decision.
    fn act(&mut self, decision: Decision, probe: &Probe<'_>, cx: &mut Cx<'_>) -> Step<Value> {
        match decision {
            Decision::Wait => Step::Wait,
            Decision::Opened => self.open_settled(probe),
            Decision::Failed(reason) => self.open_failed(probe.tick, format_args!("{reason}")),
            Decision::Trade(keeper) => {
                self.press_trade(probe, &keeper, cx);
                Step::Wait
            }
            Decision::Walk(tile) => {
                trace(
                    "walk",
                    probe.tick,
                    format_args!("unreachable keeper at {tile:?}"),
                );
                match Resilient::new(tile, 1, KEEPER_WALK_MS, Some(HOP_ATTEMPTS), false).start(cx) {
                    Ok(walk) => {
                        self.walk = Some(Box::new(walk));
                        Step::Wait
                    }
                    Err(arrived) => {
                        let decision = self.open.walked(arrived, || probe.keeper.resolve());
                        self.act(decision, probe, cx)
                    }
                }
            }
        }
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
        if self.phase == Phase::ReadyBatch {
            let held = count_of(probe.inv, &self.name);
            let delta = match self.kind {
                Kind::Buy => (held - self.held_now).max(0),
                _ => (self.held_now - held).max(0),
            };
            self.transferred = self.transferred.saturating_add(delta);
            self.held_now = held;
            if self.transferred >= self.requested {
                return self.done(true);
            }
        }
        // A successful last click can remove the row entirely. Count the
        // posted backpack change before checking for a row to click again.
        let row = find_row(container, &self.name).cloned();
        match self.phase {
            Phase::WaitBatch => {
                let delta = self.batch_delta(probe);
                self.batch_delta = self.batch_delta.max(delta);
                if self.batch_delta == 0 {
                    return if cx.clock().bound_reached() {
                        self.done(false)
                    } else {
                        Step::Wait
                    };
                }
                let transferred = self.transferred.saturating_add(self.batch_delta);
                if transferred >= self.requested {
                    self.transferred = transferred;
                    self.batch_delta = 0;
                    return self.done(true);
                }
                self.transferred = transferred;
                self.batch_delta = 0;
                self.held_now = count_of(probe.inv, &self.name);
                if self.pick {
                    // The frozen loop asks `pick` again before every batch.
                    self.candidates = container
                        .iter()
                        .filter(|row| row.name.eq_ignore_ascii_case(&self.name))
                        .cloned()
                        .collect();
                    if self.candidates.is_empty() {
                        return self.done(false);
                    }
                    if self.tick_spent() {
                        self.phase = Phase::ReadyBatch;
                        return Step::Wait;
                    }
                    self.phase = Phase::Pick;
                    return self.pick_step(probe, None, cx);
                }
                let Some(row) = row else {
                    return self.done(false);
                };
                // The frozen `Shop.buy` breaks out on a stock row already at zero.
                if self.kind == Kind::Buy && row.count <= 0 {
                    return self.done(false);
                }
                if self.tick_spent() {
                    self.phase = Phase::ReadyBatch;
                    return Step::Wait;
                }
                self.send_batch(&row, cx)
            }
            Phase::ReadyBatch => {
                if self.tick_spent() {
                    return Step::Wait;
                }
                if self.pick {
                    self.candidates = container
                        .iter()
                        .filter(|row| row.name.eq_ignore_ascii_case(&self.name))
                        .cloned()
                        .collect();
                    self.phase = Phase::Pick;
                    return self.pick_step(probe, None, cx);
                }
                let Some(row) = row else {
                    return self.done(false);
                };
                if self.kind == Kind::Buy && row.count <= 0 {
                    return self.done(false);
                }
                self.send_batch(&row, cx)
            }
            Phase::WaitBoundary | Phase::Pick => self.done(false),
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
    tick: Option<u64>,
    session: Session,
    chat_seq: i32,
    cant_reach_seq: i32,
    ingame: bool,
    shop_open: bool,
    has_stock: bool,
    stock: &'a [Row],
    player: Option<&'a [Row]>,
    inv: &'a [Row],
    keeper: KeeperFact<'a>,
}

impl Probe<'_> {
    /// The facts the shared open core decides from.
    fn facts(&self) -> Facts {
        Facts {
            tick: self.tick,
            session: self.session,
            chat_seq: self.chat_seq,
            cant_reach_seq: self.cant_reach_seq,
            usable: self.shop_open && self.has_stock && !self.stock.is_empty(),
            trade_taken: self
                .tick
                .is_some_and(|tick| trade_emitted_on(observed::with(Scene::epoch), tick)),
        }
    }
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

/// A positive whole count; a JS number beyond int32 reaches Rust as a float.
fn requested(qty: &Value) -> Option<i32> {
    qty.as_i64()
        .or_else(|| qty.as_f64().filter(|n| n.fract() == 0.0).map(|n| n as i64))
        .filter(|qty| *qty >= 1)
        .map(|qty| qty as i32)
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
    use crate::shop_open::{KeeperRoute, OPEN_ATTEMPTS, OPEN_SLACK_TICKS};
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

    fn npc_row(name: &str, ops: &[&str], index: i32, x: i32, distance: i32) -> observed::EntityRow {
        observed::EntityRow {
            index,
            name: Some(name.into()),
            x,
            z: 5,
            level: 0,
            distance,
            actions: ops.iter().map(|op| (*op).into()).collect(),
            ..observed::EntityRow::default()
        }
    }

    #[test]
    fn choose_keeper_needs_the_npc_and_its_trade_op() {
        let rows = [
            npc_row("Guard", &["Attack"], 1, 6, 1),
            npc_row("Shop keeper", &["Talk-to"], 2, 6, 1),
            npc_row("Shop keeper", &["Talk-to", "Trade"], 3, 9, 4),
            npc_row("Shop keeper", &["Talk-to", "Trade"], 4, 7, 2),
        ];
        assert!(choose_keeper(&rows, "Guard", None).is_none());
        assert!(choose_keeper(&[], "Shop keeper", None).is_none());
        // No live flood: the nearest Trade row by straight-line distance.
        let nearest = choose_keeper(&rows, " shop keeper ", None).unwrap();
        assert_eq!((nearest.index, nearest.route), (4, KeeperRoute::Unknown(2)));
        assert_eq!(nearest.action, "Trade");
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
            tick: Some(1),
            session: Session::default(),
            chat_seq: 0,
            cant_reach_seq: 0,
            ingame: true,
            shop_open: true,
            has_stock: true,
            stock,
            player,
            inv,
            keeper: KeeperFact::Given(None),
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
            Kind::Buy | Kind::Sell | Kind::SellAll => shop.transfer_step(probe, &mut cx),
        };
        let out = match step {
            Step::Wait => None,
            Step::Done(value) => Some(value),
            _ => panic!("the shop neither calls back nor fails"),
        };
        (out, ops)
    }

    #[test]
    fn sell_all_caps_clicks_even_while_shop_still_has_stock() {
        let mut rt = shop(Kind::SellAll, "Vial", i32::MAX, Phase::WaitBatch);
        rt.clicks = SELL_ALL_CLICKS - 1;
        rt.batch_baseline = 100;
        rt.held_now = 100;
        rt.last_dispatch_tick = Some(0);
        let mut clock = InstantTaskClock::new();
        let pack = [row("Vial", 100, 1)];
        let inv = [row("Vial", 90, 1)];
        observed::on_reset();
        observed::post(1, |_| {});
        let (out, ops) = step(&mut rt, &mut clock, &probe(&[], Some(&pack), &inv));
        assert!(out.is_none());
        assert_eq!(ops.len(), 1);
        assert_eq!(rt.clicks, SELL_ALL_CLICKS);
        let after = [row("Vial", 80, 1)];
        let (out, ops) = step(&mut rt, &mut clock, &probe(&[], Some(&pack), &after));
        assert!(out.is_none());
        assert!(
            ops.is_empty(),
            "same-tick snapshot cannot dispatch batch 41"
        );
        observed::post(2, |_| {});
        let (out, ops) = step(&mut rt, &mut clock, &probe(&[], Some(&pack), &after));
        assert_eq!(out, Some(json!(20)));
        assert!(ops.is_empty());
    }

    #[test]
    fn sell_all_counts_the_final_stack_when_the_row_disappears() {
        let mut rt = shop(Kind::SellAll, "Vial", i32::MAX, Phase::WaitBatch);
        rt.batch_baseline = 7;
        let mut clock = armed(SETTLE_MS);
        let empty_pack: [Row; 0] = [];
        let post = probe(&[], Some(&empty_pack), &[]);
        assert_eq!(step(&mut rt, &mut clock, &post), (Some(json!(7)), vec![]));
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
        // The bounded window itself is enough to classify a batch with no
        // observed movement as stalled; no extra game tick is manufactured.
        expire(&mut clock);
        assert_eq!(step(&mut rt, &mut clock, &probe), (Some(json!(0)), vec![]));
    }

    fn opener() -> Shop {
        observed::on_reset();
        let mut rt = shop(Kind::Open, "Shop keeper", 0, Phase::WaitBoundary);
        rt.open = OpenCore::default();
        rt
    }

    fn keeper(route: KeeperRoute) -> Keeper {
        Keeper {
            name: "Shop keeper".into(),
            action: "Trade".into(),
            index: 3,
            tile: WorldTile {
                x: 3222,
                z: 3222,
                level: 0,
            },
            route,
        }
    }

    fn trade() -> InteractReq {
        InteractReq::Npc {
            name: "Shop keeper".into(),
            action: "Trade".into(),
            index: Some(3),
        }
    }

    /// A real tick with the shop closed and `keeper` posted.
    fn closed_at(tick: u64, keeper: &Keeper) -> Probe<'_> {
        Probe {
            tick: Some(tick),
            keeper: KeeperFact::Given(Some(keeper)),
            shop_open: false,
            has_stock: false,
            ..probe(&[], None, &[])
        }
    }

    /// A real tick with the shop modal up over `stock`.
    fn open_at<'a>(tick: u64, keeper: &'a Keeper, stock: &'a [Row]) -> Probe<'a> {
        Probe {
            tick: Some(tick),
            keeper: KeeperFact::Given(Some(keeper)),
            ..probe(stock, None, &[])
        }
    }

    #[test]
    fn a_long_walk_gets_one_trade_until_its_route_deadline() {
        let mut rt = opener();
        let mut clock = InstantTaskClock::new();
        let walk = keeper(KeeperRoute::Steps(30));
        assert_eq!(
            step(&mut rt, &mut clock, &closed_at(1, &walk)),
            (None, vec![trade()])
        );
        let deadline = 1 + 30 + OPEN_SLACK_TICKS;
        for tick in 2..=deadline {
            assert_eq!(
                step(&mut rt, &mut clock, &closed_at(tick, &walk)),
                (None, vec![]),
                "no second Trade on tick {tick} before the deadline"
            );
        }
        assert_eq!(
            step(&mut rt, &mut clock, &closed_at(deadline + 1, &walk)),
            (None, vec![trade()]),
            "the expired attempt is followed by exactly one new Trade"
        );
    }

    #[test]
    fn a_modal_at_the_deadline_tick_settles_on_a_later_tick_not_a_trade_tick() {
        let mut rt = opener();
        let mut clock = InstantTaskClock::new();
        let stock = [row("Shantay pass", 100, 15)];
        let walk = keeper(KeeperRoute::Steps(2));
        let deadline = 1 + 2 + OPEN_SLACK_TICKS;
        assert_eq!(
            step(&mut rt, &mut clock, &closed_at(1, &walk)),
            (None, vec![trade()])
        );
        // A usable page on the Trade tick itself is not this Trade's shop.
        assert!(!rt.open.ready(&open_at(1, &walk, &stock).facts()));
        for tick in 2..deadline {
            assert_eq!(
                step(&mut rt, &mut clock, &closed_at(tick, &walk)),
                (None, vec![])
            );
        }
        // The modal lands on the deadline tick; its stock page is not posted yet.
        assert_eq!(
            step(&mut rt, &mut clock, &open_at(deadline, &walk, &[])),
            (None, vec![])
        );
        // The page lands with the next tick: settle before any expiry check.
        assert_eq!(
            step(&mut rt, &mut clock, &open_at(deadline + 1, &walk, &stock)),
            (Some(json!(true)), vec![])
        );
    }

    #[test]
    fn cant_reach_fails_that_attempt_at_once() {
        let mut rt = opener();
        let mut clock = InstantTaskClock::new();
        let walk = keeper(KeeperRoute::Steps(9));
        // An older "I can't reach that!" is before this Trade's mark.
        let before = Probe {
            chat_seq: 4,
            cant_reach_seq: 4,
            ..closed_at(1, &walk)
        };
        assert_eq!(step(&mut rt, &mut clock, &before), (None, vec![trade()]));
        let old_line = Probe {
            chat_seq: 4,
            cant_reach_seq: 4,
            ..closed_at(2, &walk)
        };
        assert_eq!(step(&mut rt, &mut clock, &old_line), (None, vec![]));
        let mut seq = 4;
        for tick in 3..=4 {
            seq += 1;
            let refused = Probe {
                chat_seq: seq,
                cant_reach_seq: seq,
                ..closed_at(tick, &walk)
            };
            assert_eq!(
                step(&mut rt, &mut clock, &refused),
                (None, vec![trade()]),
                "a fresh can't-reach ends the attempt on tick {tick}"
            );
        }
        let refused = Probe {
            chat_seq: seq + 1,
            cant_reach_seq: seq + 1,
            ..closed_at(5, &walk)
        };
        assert_eq!(
            step(&mut rt, &mut clock, &refused),
            (Some(json!(false)), vec![]),
            "the third refused attempt ends the call without a fourth Trade"
        );
    }

    #[test]
    fn without_a_live_flood_the_straight_line_distance_sets_the_deadline() {
        let mut rt = opener();
        let mut clock = InstantTaskClock::new();
        let unknown = keeper(KeeperRoute::Unknown(5));
        assert_eq!(
            step(&mut rt, &mut clock, &closed_at(1, &unknown)),
            (None, vec![trade()])
        );
        for tick in 2..=1 + 5 + OPEN_SLACK_TICKS {
            assert_eq!(
                step(&mut rt, &mut clock, &closed_at(tick, &unknown)),
                (None, vec![])
            );
        }
        assert_eq!(
            step(
                &mut rt,
                &mut clock,
                &closed_at(2 + 5 + OPEN_SLACK_TICKS, &unknown)
            ),
            (None, vec![trade()])
        );
    }

    #[test]
    fn three_expired_attempts_resolve_false_without_a_trade_on_the_last_expiry() {
        let mut rt = opener();
        let mut clock = InstantTaskClock::new();
        let here = keeper(KeeperRoute::Steps(0));
        let mut tick = 1;
        let mut trades = 0;
        let result = loop {
            let (out, ops) = step(&mut rt, &mut clock, &closed_at(tick, &here));
            if !ops.is_empty() {
                assert_eq!(ops, vec![trade()]);
                trades += 1;
            }
            if let Some(out) = out {
                assert!(ops.is_empty(), "the failing tick sends nothing");
                break out;
            }
            tick += 1;
            assert!(tick < 3 * (OPEN_SLACK_TICKS + 2), "bounded");
        };
        assert_eq!(result, json!(false));
        assert_eq!(trades, OPEN_ATTEMPTS);
        assert_eq!(tick, 1 + u64::from(OPEN_ATTEMPTS) * (OPEN_SLACK_TICKS + 1));
    }

    #[test]
    fn standing_next_to_the_keeper_gets_one_trade_not_two() {
        let mut rt = opener();
        let mut clock = InstantTaskClock::new();
        let stock = [row("Shantay pass", 100, 15)];
        let here = keeper(KeeperRoute::Steps(0));
        assert_eq!(
            step(&mut rt, &mut clock, &closed_at(1, &here)),
            (None, vec![trade()])
        );
        // The engine opens the shop a few cycles later (keeper wandered):
        // stationary ticks with no face posted are not an expiry.
        for tick in 2..=5 {
            assert_eq!(
                step(&mut rt, &mut clock, &closed_at(tick, &here)),
                (None, vec![])
            );
        }
        assert_eq!(
            step(&mut rt, &mut clock, &open_at(6, &here, &stock)),
            (Some(json!(true)), vec![])
        );
    }

    #[test]
    fn a_session_change_ends_the_outstanding_attempt() {
        let mut rt = opener();
        let mut clock = InstantTaskClock::new();
        let walk = keeper(KeeperRoute::Steps(40));
        assert_eq!(
            step(&mut rt, &mut clock, &closed_at(9, &walk)),
            (None, vec![trade()])
        );
        let relogged = Probe {
            session: Session { epoch: 0, login: 7 },
            ..closed_at(10, &walk)
        };
        assert_eq!(
            step(&mut rt, &mut clock, &relogged),
            (None, vec![trade()]),
            "a logout post drops the old Trade with its session"
        );
        // A reconnect resets the scene; its ticks may restart below the
        // Trade tick and must not hold the attempt open.
        let reconnected = Probe {
            session: Session { epoch: 1, login: 0 },
            ..closed_at(2, &walk)
        };
        assert_eq!(
            step(&mut rt, &mut clock, &reconnected),
            (None, vec![trade()])
        );
    }

    #[test]
    fn an_open_modal_without_a_page_trades_only_after_the_next_tick() {
        let mut rt = opener();
        rt.open = OpenCore::awaiting_page(&probe(&[], None, &[]).facts());
        let mut clock = InstantTaskClock::new();
        let here = keeper(KeeperRoute::Steps(0));
        assert_eq!(
            step(&mut rt, &mut clock, &open_at(2, &here, &[])),
            (None, vec![])
        );
        assert_eq!(
            step(&mut rt, &mut clock, &open_at(3, &here, &[])),
            (None, vec![trade()])
        );
    }

    #[test]
    fn an_unreachable_keeper_with_no_walk_fails_without_a_trade() {
        // No posted player tile: the nav walk cannot start, so the open
        // fails instead of clicking an unreachable keeper.
        let mut rt = opener();
        let mut clock = InstantTaskClock::new();
        let blocked = keeper(KeeperRoute::Unreachable);
        assert_eq!(
            step(&mut rt, &mut clock, &closed_at(1, &blocked)),
            (Some(json!(false)), vec![])
        );
    }

    #[test]
    fn a_vanished_keeper_ends_the_call_after_an_expired_attempt() {
        let mut rt = opener();
        let mut clock = InstantTaskClock::new();
        let here = keeper(KeeperRoute::Steps(0));
        assert_eq!(
            step(&mut rt, &mut clock, &closed_at(1, &here)),
            (None, vec![trade()])
        );
        let gone = Probe {
            keeper: KeeperFact::Given(None),
            ..closed_at(2 + OPEN_SLACK_TICKS, &here)
        };
        assert_eq!(
            step(&mut rt, &mut clock, &gone),
            (Some(json!(false)), vec![])
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
    fn a_depleted_stock_row_counts_the_last_batch_before_stopping() {
        let mut rt = shop(Kind::Buy, "Feather", 25, Phase::WaitBatch);
        rt.transferred = 10;
        rt.batch_baseline = 10;
        let mut clock = InstantTaskClock::new();
        let empty = [row("Feather", 0, 3)];
        let inv = [row("Feather", 12, 1)];
        assert_eq!(
            step(&mut rt, &mut clock, &probe(&empty, None, &inv)),
            (Some(json!(12)), vec![]),
            "last batch movement is counted before stock depletion"
        );
    }

    #[test]
    fn snapshot_progress_cannot_complete_after_shop_closes() {
        let mut rt = shop(Kind::Buy, "Feather", 10, Phase::WaitBatch);
        rt.batch_baseline = 4;
        let mut clock = armed(SETTLE_MS);
        let stock = [row("Feather", 20, 3)];
        let inv = [row("Feather", 14, 1)];
        let closed = Probe {
            shop_open: false,
            ..probe(&stock, None, &inv)
        };
        assert_eq!(
            step(&mut rt, &mut clock, &closed),
            (Some(json!(0)), vec![]),
            "a closed shop cannot authorize fresh transfer progress"
        );
    }

    #[test]
    fn ready_batch_counts_later_snapshot_progress_before_dispatching() {
        let mut rt = shop(Kind::Buy, "Feather", 25, Phase::ReadyBatch);
        rt.transferred = 20;
        rt.held_now = 20;
        rt.last_dispatch_tick = Some(1);
        let mut clock = InstantTaskClock::new();
        let stock = [row("Feather", 90, 3)];
        let inv = [row("Feather", 25, 1)];
        observed::on_reset();
        observed::post(1, |_| {});
        assert_eq!(
            step(&mut rt, &mut clock, &probe(&stock, None, &inv)),
            (Some(json!(25)), vec![])
        );
    }

    #[test]
    fn a_served_batch_counts_snapshot_progress_and_defers_its_remainder() {
        let mut rt = shop(Kind::Buy, "Feather", 25, Phase::WaitBatch);
        rt.transferred = 10;
        rt.batch_baseline = 10;
        rt.last_dispatch_tick = Some(1);
        let mut clock = armed(SETTLE_MS);
        let stock = [row("Feather", 90, 3)];
        let inv = [row("Feather", 20, 1)];
        let probe = probe(&stock, None, &inv);
        observed::on_reset();
        observed::post(1, |_| {});
        // This same-tick snapshot contributes the moved quantity but cannot
        // dispatch a second batch.
        assert_eq!(step(&mut rt, &mut clock, &probe), (None, vec![]));
        assert_eq!(rt.transferred, 20);
        assert_eq!(rt.phase, Phase::ReadyBatch);

        // The next observed game tick can dispatch the remaining five.
        observed::post(2, |_| {});
        let (out, ops) = step(&mut rt, &mut clock, &probe);
        assert_eq!(out, None);
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
        assert_eq!(step(&mut rt, &mut clock, &probe), (Some(json!(15)), vec![]));
    }

    #[test]
    fn sell_plans_against_the_shop_player_pack_and_counts_inventory_losses() {
        let mut rt = shop(Kind::Sell, "Vial", 10, Phase::WaitBatch);
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
