//! Pollable settle evidence (the m8aq `Settle`/`Evidence`/`Outcome`):
//! one `poll(now)` runs one watch step against the `before` context the
//! settle was built with, then reports a matched arm, an expiry (a
//! disconnect, or the tick/ms budget lapsed), or `None` while still
//! watching. The host drives the polls per tick; nothing here blocks.
//! `Refused` is produced by `Interactions`, never by `poll`.

use crate::interact::SendReason;
use crate::query::{ChatQueryExt, EntityQueryExt, ItemQueryExt, Query, StatQueryExt};
use crate::snapshot::{
    ActorKind, ActorTargetView, ItemContainer, ItemView, LocView, ReadContext, WorldTile,
};

/// Evidence an arm fired: `(now, before)` snapshot reads, `true` when the
/// arm's condition holds on the current watch step. Each predicate is a
/// closure closing over its parameters (the m8aq `Evidence`); the
/// unparameterized predicates are `'static` closures.
pub type Evidence<'a> = Box<dyn Fn(&ReadContext<'_>, &ReadContext<'_>) -> bool + 'a>;

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
/// (the m8aq `SettleOptions`). The `before` context passed to
/// [`Settle::new`] is the caller's pre-watch read.
pub struct SettleOptions<'a> {
    pub arms: &'a [(&'a str, Evidence<'a>)],
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

// --- evidence predicates ---------------------------------------------------

/// Arrived: the local player is within `radius` (Chebyshev) of `tile`.
/// The position is the snapshot's canonical world tile (base + route
/// head), the same source `Traveller::follow`'s `here()` and the runner's
/// `arrived` proof read — so a hop and its settle arm can never disagree
/// about where the player is.
pub fn arrived(tile: WorldTile, radius: i32) -> Evidence<'static> {
    Box::new(move |now: &ReadContext<'_>, _before: &ReadContext<'_>| {
        let Some(here) = now.world_tile() else {
            return false;
        };
        if here.level != tile.level {
            return false;
        }
        (here.x - tile.x).abs().max((here.z - tile.z).abs()) <= radius
    })
}

/// A container's `item_id` total moved by at least (positive `change`) or
/// at most (negative `change`) the given amount. The `Widget` container
/// falls back to inventory, the m8aq `containerQuery` default arm.
pub fn item_delta(item_id: i32, change: i32, container: ItemContainer) -> Evidence<'static> {
    Box::new(move |now: &ReadContext<'_>, before: &ReadContext<'_>| {
        let moved = item_total(now, container, item_id) - item_total(before, container, item_id);
        if change >= 0 {
            moved >= change
        } else {
            moved <= change
        }
    })
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
pub fn xp_gained(skill_index: i32, at_least: i32) -> Evidence<'static> {
    Box::new(move |now: &ReadContext<'_>, before: &ReadContext<'_>| {
        let xp = |ctx: &ReadContext<'_>| {
            Query::new(ctx.stats())
                .with_index(&[skill_index])
                .first()
                .map(|s| s.xp)
                .unwrap_or(0)
        };
        xp(now) - xp(before) >= at_least
    })
}

/// `target` is engaged: the local player faces it, or its health dropped
/// since `before`.
pub fn engaged(target: ActorTargetView) -> Evidence<'static> {
    Box::new(move |now: &ReadContext<'_>, before: &ReadContext<'_>| {
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
    })
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
pub fn modal_opened(root: Option<i32>) -> Evidence<'static> {
    Box::new(move |now: &ReadContext<'_>, _before: &ReadContext<'_>| {
        let modals = now.modals();
        match root {
            None => {
                modals.main != -1 || modals.side != -1 || modals.chat != -1 || modals.tutorial != -1
            }
            Some(id) => {
                modals.main == id
                    || modals.side == id
                    || modals.chat == id
                    || modals.tutorial == id
            }
        }
    })
}

/// No modal root (`None`) or none of the four roots equal `id`.
pub fn modal_closed(root: Option<i32>) -> Evidence<'static> {
    Box::new(move |now: &ReadContext<'_>, _before: &ReadContext<'_>| {
        let modals = now.modals();
        match root {
            None => {
                modals.main == -1
                    && modals.side == -1
                    && modals.chat == -1
                    && modals.tutorial == -1
            }
            Some(id) => {
                modals.main != id
                    && modals.side != id
                    && modals.chat != id
                    && modals.tutorial != id
            }
        }
    })
}

/// `target`'s `action` no longer appears on the loc at its tile (or the
/// loc is gone entirely).
pub fn option_gone(target: &LocView, action: &str) -> Evidence<'static> {
    let id = target.id;
    let layer = target.layer;
    let tile = target.tile;
    let wanted = action.trim().to_ascii_lowercase();
    Box::new(move |now: &ReadContext<'_>, _before: &ReadContext<'_>| {
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
    })
}

/// A chat line newer than the `before` read contains one of `phrases`
/// (trimmed, case-insensitive).
pub fn said(phrases: &[&str]) -> Evidence<'static> {
    let phrases: Vec<String> = phrases.iter().map(|p| (*p).to_string()).collect();
    Box::new(move |now: &ReadContext<'_>, before: &ReadContext<'_>| {
        let phrases: Vec<&str> = phrases.iter().map(|s| s.as_str()).collect();
        let after = Query::new(before.chat()).latest_sequence();
        Query::new(now.chat())
            .since(after)
            .text_contains(&phrases)
            .exists()
    })
}

/// A walk was refused: a map flag was set at `before` and dropped at
/// `now` while the player is not on the flag.
pub fn server_refused() -> Evidence<'static> {
    Box::new(|now: &ReadContext<'_>, before: &ReadContext<'_>| {
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
    })
}

/// The scene is ready: `scene_state == 2` with a nonzero build base.
pub fn scene_ready() -> Evidence<'static> {
    Box::new(|now: &ReadContext<'_>, _before: &ReadContext<'_>| {
        now.scene_state() == crate::interact::SCENE_READY && now.scene().base_x != 0
    })
}

/// Any inventory slot's id/count changed since `before`.
pub fn inventory_changed() -> Evidence<'static> {
    Box::new(|now: &ReadContext<'_>, before: &ReadContext<'_>| {
        fn describe(ctx: &ReadContext<'_>) -> Vec<(i32, i32, i32)> {
            ctx.inventory()
                .iter()
                .map(|i| (i.slot, i.def.id, i.count))
                .collect()
        }
        describe(now) != describe(before)
    })
}
