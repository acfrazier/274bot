//! Pollable settle evidence (the m8aq `Settle`/`Evidence`/`Outcome`):
//! one `poll(now)` runs one watch step against the `before` context the
//! settle was built with, then reports a matched arm, an expiry (a
//! disconnect, or the tick/ms budget lapsed), or `None` while still
//! watching. The host drives the polls per tick; nothing here blocks.
//! `Refused` is produced by `Interactions`, never by `poll`.

use std::cell::{Cell, RefCell};

use crate::interact::SendReason;
use crate::query::{ChatQueryExt, EntityQueryExt, ItemQueryExt, Query, StatQueryExt};
use crate::snapshot::{
    ActorKind, ActorTargetView, ItemContainer, ItemView, LocLayer, LocView, ReadContext,
    WorldTile,
};

/// Evidence an arm fired: `(now, before)` snapshot reads, `true` when the
/// arm's condition holds on the current watch step. The predicates are
/// plain `fn` pointers (Copy, slice-storable), so each parameterized
/// predicate's arguments live in the thread-local slot its constructor
/// registers; a settle's arms are built immediately before `Settle::new`
/// and one settle is watched at a time per thread, so the slots stay
/// stable for the whole watch.
pub type Evidence = fn(&ReadContext<'_>, &ReadContext<'_>) -> bool;

/// The outcome of a settle watch. `Refused` is produced by `Interactions`
/// (a refused send never starts a watch); `poll` reports `Matched` when an
/// arm fires and `Expired` when the watch lapses.
pub enum Outcome<'a> {
    Refused {
        reason: SendReason,
        tick: u64,
    },
    Matched {
        arm: &'a str,
        now: ReadContext<'a>,
        before: ReadContext<'a>,
        tick: u64,
    },
    Expired {
        now: ReadContext<'a>,
        before: ReadContext<'a>,
        tick: u64,
    },
}

/// One settle watch: the arms to check per step plus the tick/ms budget
/// (the m8aq `SettleOptions`). `since` is the caller's pre-watch read;
/// the `before` context passed to [`Settle::new`] is resolved from it.
pub struct SettleOptions<'a> {
    pub arms: &'a [(&'a str, Evidence)],
    pub since: Option<ReadContext<'a>>,
    pub budget_ticks: u32,
    pub budget_ms: Option<u64>,
}

/// A pollable settle: one [`Settle::poll`] step per call. `before` is
/// fixed at construction; the watch ends with the first matched arm, a
/// disconnect, or the tick/ms budget.
pub struct Settle<'a> {
    options: SettleOptions<'a>,
    before: ReadContext<'a>,
    start_tick: u64,
    deadline_ms: Option<u64>,
}

/// The live-tick length and the wall-clock backstop multiplier (the m8aq
/// `LIVE_TICK_MS`/`BACKSTOP_MULTIPLIER`).
const LIVE_TICK_MS: u64 = 600;
const BACKSTOP_MULTIPLIER: u64 = 4;

impl<'a> Settle<'a> {
    pub fn new(options: SettleOptions<'a>, before: ReadContext<'a>) -> Self {
        let start_tick = before.tick() as u64;
        let budget_ms = options
            .budget_ms
            .unwrap_or(options.budget_ticks as u64 * LIVE_TICK_MS * BACKSTOP_MULTIPLIER);
        let deadline_ms = now_ms().map(|now| now + budget_ms);
        Settle {
            options,
            before,
            start_tick,
            deadline_ms,
        }
    }

    /// Run one watch step against `now`: check each arm against
    /// `(now, before)`, then the disconnect / tick-ms budget. `None`
    /// while still watching.
    pub fn poll(&mut self, now: ReadContext<'a>) -> Option<Outcome<'a>> {
        let tick = now.tick() as u64;
        for (arm, evidence) in self.options.arms {
            if evidence(&now, &self.before) {
                return Some(Outcome::Matched {
                    arm,
                    now,
                    before: self.before,
                    tick,
                });
            }
        }
        if !now.attached() || !now.ingame() {
            return Some(Outcome::Expired {
                now,
                before: self.before,
                tick,
            });
        }
        let over_ticks = (now.tick() as u64).saturating_sub(self.start_tick)
            >= self.options.budget_ticks as u64;
        let over_ms = self
            .deadline_ms
            .is_some_and(|deadline| now_ms().is_some_and(|now| now >= deadline));
        if over_ticks || over_ms {
            return Some(Outcome::Expired {
                now,
                before: self.before,
                tick,
            });
        }
        None
    }
}

/// Milliseconds since the Unix epoch (the m8aq `Date.now()`).
fn now_ms() -> Option<u64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as u64)
}

// --- evidence parameter slots ---------------------------------------------
//
// Each parameterized predicate registers its arguments in one thread-local
// slot and returns a fixed `fn` implementation reading that slot (see the
// `Evidence` doc note on why the arguments cannot ride inside the fn
// pointer).

const ORIGIN_TILE: WorldTile = WorldTile {
    x: 0,
    z: 0,
    level: 0,
};

thread_local! {
    static ARRIVED: Cell<(WorldTile, i32)> = const { Cell::new((ORIGIN_TILE, 0)) };
    static ITEM_DELTA: Cell<(i32, i32, ItemContainer)> =
        const { Cell::new((0, 0, ItemContainer::Inventory)) };
    static XP_GAINED: Cell<(i32, i32)> = const { Cell::new((0, 1)) };
    static ENGAGED: Cell<ActorTargetView> =
        const { Cell::new(ActorTargetView { kind: ActorKind::Npc, index: 0 }) };
    static MODAL_OPENED: Cell<Option<i32>> = const { Cell::new(None) };
    static MODAL_CLOSED: Cell<Option<i32>> = const { Cell::new(None) };
    static OPTION_GONE: RefCell<(i32, LocLayer, WorldTile, String)> =
        const { RefCell::new((0, LocLayer::Wall, ORIGIN_TILE, String::new())) };
    static SAID: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

// --- evidence predicates ---------------------------------------------------

/// Arrived: the local player is within `radius` (Chebyshev) of `tile`.
pub fn arrived(tile: WorldTile, radius: i32) -> Evidence {
    ARRIVED.with(|slot| slot.set((tile, radius)));
    arrived_impl
}

fn arrived_impl(now: &ReadContext<'_>, _before: &ReadContext<'_>) -> bool {
    let (tile, radius) = ARRIVED.with(|slot| slot.get());
    let Some(here) = now.local_player() else {
        return false;
    };
    let here = here.player.actor.tile;
    if here.level != tile.level {
        return false;
    }
    (here.x - tile.x).abs().max((here.z - tile.z).abs()) <= radius
}

/// A container's `item_id` total moved by at least (positive `change`) or
/// at most (negative `change`) the given amount. The `Widget` container
/// falls back to inventory, the m8aq `containerQuery` default arm.
pub fn item_delta(item_id: i32, change: i32, container: ItemContainer) -> Evidence {
    ITEM_DELTA.with(|slot| slot.set((item_id, change, container)));
    item_delta_impl
}

fn item_delta_impl(now: &ReadContext<'_>, before: &ReadContext<'_>) -> bool {
    let (item_id, change, container) = ITEM_DELTA.with(|slot| slot.get());
    let moved = item_total(now, container, item_id) - item_total(before, container, item_id);
    if change >= 0 {
        moved >= change
    } else {
        moved <= change
    }
}

fn item_total(ctx: &ReadContext<'_>, container: ItemContainer, item_id: i32) -> i32 {
    let items = container_items(ctx, container);
    Query::new(items).with_id(&[item_id]).total()
}

fn container_items<'a>(ctx: &'a ReadContext<'a>, container: ItemContainer) -> &'a [ItemView] {
    match container {
        ItemContainer::Equipment => ctx.equipment(),
        ItemContainer::Bank => ctx.bank(),
        ItemContainer::BankSide => ctx.bank_side_items(),
        ItemContainer::TradeMyOffer => ctx.trade_my_offer(),
        ItemContainer::TradeTheirOffer => ctx.trade_their_offer(),
        ItemContainer::TradeSidePack => ctx.trade_side_pack(),
        ItemContainer::Inventory | ItemContainer::Widget => ctx.inventory(),
    }
}

/// `skill_index` XP gained at least `at_least`.
pub fn xp_gained(skill_index: i32, at_least: i32) -> Evidence {
    XP_GAINED.with(|slot| slot.set((skill_index, at_least)));
    xp_gained_impl
}

fn xp_gained_impl(now: &ReadContext<'_>, before: &ReadContext<'_>) -> bool {
    let (skill_index, at_least) = XP_GAINED.with(|slot| slot.get());
    let xp = |ctx: &ReadContext<'_>| {
        Query::new(ctx.stats())
            .with_index(&[skill_index])
            .first()
            .map(|s| s.xp)
            .unwrap_or(0)
    };
    xp(now) - xp(before) >= at_least
}

/// `target` is engaged: the local player faces it, or its health dropped
/// since `before`.
pub fn engaged(target: ActorTargetView) -> Evidence {
    ENGAGED.with(|slot| slot.set(target));
    engaged_impl
}

fn engaged_impl(now: &ReadContext<'_>, before: &ReadContext<'_>) -> bool {
    let target = ENGAGED.with(|slot| slot.get());
    if let Some(me) = now.local_player() {
        if let Some(t) = me.player.actor.target {
            if t.index == target.index && t.kind == target.kind {
                return true;
            }
        }
    }
    let then = target_health(before, target);
    let live = target_health(now, target);
    matches!((then, live), (Some(t), Some(l)) if l < t)
}

fn target_health(ctx: &ReadContext<'_>, target: ActorTargetView) -> Option<i32> {
    match target.kind {
        ActorKind::Npc => Query::new(ctx.npcs())
            .with_index(&[target.index])
            .first()
            .map(|n| n.health),
        ActorKind::Player => Query::new(ctx.players())
            .with_index(&[target.index])
            .first()
            .map(|p| p.actor.health),
    }
}

/// Any modal root (`None`) or the named root (`Some(id)`) opened among the
/// four roots.
pub fn modal_opened(root: Option<i32>) -> Evidence {
    MODAL_OPENED.with(|slot| slot.set(root));
    modal_opened_impl
}

fn modal_opened_impl(now: &ReadContext<'_>, _before: &ReadContext<'_>) -> bool {
    let root = MODAL_OPENED.with(|slot| slot.get());
    let modals = now.modals();
    match root {
        None => {
            modals.main != -1 || modals.side != -1 || modals.chat != -1 || modals.tutorial != -1
        }
        Some(id) => {
            modals.main == id || modals.side == id || modals.chat == id || modals.tutorial == id
        }
    }
}

/// No modal root (`None`) or none of the four roots equal `id`.
pub fn modal_closed(root: Option<i32>) -> Evidence {
    MODAL_CLOSED.with(|slot| slot.set(root));
    modal_closed_impl
}

fn modal_closed_impl(now: &ReadContext<'_>, _before: &ReadContext<'_>) -> bool {
    let root = MODAL_CLOSED.with(|slot| slot.get());
    let modals = now.modals();
    match root {
        None => {
            modals.main == -1 && modals.side == -1 && modals.chat == -1 && modals.tutorial == -1
        }
        Some(id) => {
            modals.main != id && modals.side != id && modals.chat != id && modals.tutorial != id
        }
    }
}

/// `target`'s `action` no longer appears on the loc at its tile (or the
/// loc is gone entirely).
pub fn option_gone(target: &LocView, action: &str) -> Evidence {
    OPTION_GONE.with(|slot| {
        *slot.borrow_mut() = (
            target.id,
            target.layer,
            target.tile,
            action.trim().to_ascii_lowercase(),
        );
    });
    option_gone_impl
}

fn option_gone_impl(now: &ReadContext<'_>, _before: &ReadContext<'_>) -> bool {
    let (id, layer, tile, wanted) = OPTION_GONE.with(|slot| slot.borrow().clone());
    let live = now.locs().iter().find(|loc| {
        loc.id == id
            && loc.layer == layer
            && loc.tile.x == tile.x
            && loc.tile.z == tile.z
            && loc.tile.level == tile.level
    });
    match live {
        None => true,
        Some(loc) => !loc.actions.iter().any(|candidate| {
            candidate
                .as_deref()
                .is_some_and(|c| c.trim().to_ascii_lowercase() == wanted)
        }),
    }
}

/// A chat line newer than the `before` read contains one of `phrases`
/// (trimmed, case-insensitive).
pub fn said(phrases: &[&str]) -> Evidence {
    SAID.with(|slot| {
        *slot.borrow_mut() = phrases.iter().map(|p| (*p).to_string()).collect();
    });
    said_impl
}

fn said_impl(now: &ReadContext<'_>, before: &ReadContext<'_>) -> bool {
    let phrases: Vec<String> = SAID.with(|slot| slot.borrow().clone());
    let phrases: Vec<&str> = phrases.iter().map(|s| s.as_str()).collect();
    let after = Query::new(before.chat()).latest_sequence();
    Query::new(now.chat())
        .since(after)
        .text_contains(&phrases)
        .exists()
}

/// A walk was refused: a map flag was set at `before` and dropped at
/// `now` while the player is not on the flag.
pub fn server_refused() -> Evidence {
    server_refused_impl
}

fn server_refused_impl(now: &ReadContext<'_>, before: &ReadContext<'_>) -> bool {
    let Some(was) = before.map_flag() else {
        return false;
    };
    if now.map_flag().is_some() {
        return false;
    }
    let Some(here) = now.world_tile() else {
        return true;
    };
    let scene = now.scene();
    if !scene.available {
        return true;
    }
    here.x - scene.base_x != was.lx || here.z - scene.base_z != was.lz
}

/// The scene is ready: `scene_state == 2` with a nonzero build base.
pub fn scene_ready() -> Evidence {
    scene_ready_impl
}

fn scene_ready_impl(now: &ReadContext<'_>, _before: &ReadContext<'_>) -> bool {
    now.scene_state() == crate::interact::SCENE_READY && now.scene().base_x != 0
}

/// Any inventory slot's id/count changed since `before`.
pub fn inventory_changed() -> Evidence {
    inventory_changed_impl
}

fn inventory_changed_impl(now: &ReadContext<'_>, before: &ReadContext<'_>) -> bool {
    fn describe(ctx: &ReadContext<'_>) -> Vec<(i32, i32, i32)> {
        ctx.inventory()
            .iter()
            .map(|i| (i.slot, i.def.id, i.count))
            .collect()
    }
    describe(now) != describe(before)
}
