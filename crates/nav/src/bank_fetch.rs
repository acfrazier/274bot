//! BankBudget (Task 8): the fetch-and-wear session that unblocks a
//! [`crate::router::find_with`] `NoPath` whose only missing gates are
//! `item_req`/`worn_req`. `find` itself stays fail-closed — the router
//! never inserts a virtual bank leg and never relaxes an edge; this
//! session is the only thing that may fetch, and the host re-runs the
//! strict search after the steps land.
//!
//! The session is a **plan**: ordered steps the host pump executes
//! through the [`crate::traveller::Traveller`] and the `api::interact`
//! bank path (open/deposit/withdraw/close/wear), plus the [`WorldState`]
//! those steps leave behind for the post-session re-find. A `worn_req`
//! alternative already carried plans a bare [`BankStep::Wear`] — no
//! bank walk. Anything the plan cannot supply (neither carried nor
//! bankable, or the relaxed diagnosis shows a skill/quest/varp gate) is
//! `None`: the caller reports `NoPath`.

use std::collections::HashSet;

use api::snapshot::WorldTile;
use client::dash3d::CollisionFlag;

use crate::collision::WorldCollision;
use crate::named_banks::{is_access_tile, AccessReach, Footprint};
use crate::pack::BankStand;
use crate::router::MissingReq;
use crate::world_state::WorldState;

/// Packed stands within this Chebyshev of each other belong to one bank
/// building: the Walk arrives at any of their access tiles and Open may use
/// any of their booths or tellers.
pub const SAME_BANK: i32 = 12;

/// The standable tiles `stand` is used from, by the access rule the C1
/// named-bank stands also use ([`is_access_tile`] over the stand's one-tile
/// [`Footprint`]): standable and never the stand's own tile. BankBudget's
/// reach is [`AccessReach::InLine`] (east/west/north/south of the stand), not
/// the named stands' diagonals: its Open tries a teller first, and a teller
/// across a booth answers only in line. A tile walled off from the stand
/// ([`wall_between`]) is not an access: Falador East's bankers stand
/// against the bank's south wall, and the Bank op from the street behind it
/// never opens the bank. Booth loc tiles are not standable
/// and most teller spawns sit in the bankers' aisle, so the stand tile
/// itself is never a walk target.
pub fn bank_access_tiles<'a>(
    collision: &'a WorldCollision,
    stand: &BankStand,
) -> impl Iterator<Item = WorldTile> + 'a {
    let at = stand.tile;
    Footprint::tile(at).access_tiles(AccessReach::InLine, move |tile| {
        collision.standable(tile) && !wall_between(collision, tile, at)
    })
}

/// Whether `tile` is one of `stand`'s [`bank_access_tiles`].
pub fn is_bank_access(collision: &WorldCollision, stand: &BankStand, tile: WorldTile) -> bool {
    is_access_tile(
        tile,
        &[Footprint::tile(stand.tile)],
        AccessReach::InLine,
        &|tile| collision.standable(tile) && !wall_between(collision, tile, stand.tile),
    )
}

/// Whether a wall stands on the shared edge of the in-line neighbours `a`
/// and `b`: the `W_*` face of either tile toward the other in the packed
/// walk word (the client stamps a wall on both). `false` for tiles that are
/// not in-line neighbours on one level.
fn wall_between(collision: &WorldCollision, a: WorldTile, b: WorldTile) -> bool {
    if a.level != b.level {
        return false;
    }
    let (a_face, b_face) = match (b.x - a.x, b.z - a.z) {
        (0, 1) => (CollisionFlag::W_N, CollisionFlag::W_S),
        (0, -1) => (CollisionFlag::W_S, CollisionFlag::W_N),
        (1, 0) => (CollisionFlag::W_E, CollisionFlag::W_W),
        (-1, 0) => (CollisionFlag::W_W, CollisionFlag::W_E),
        _ => return false,
    };
    collision.walkable_word(a.x, a.z, a.level) & a_face as u32 != 0
        || collision.walkable_word(b.x, b.z, b.level) & b_face as u32 != 0
}

/// The access tile BankBudget walks to: the nearest stand (same level,
/// then Chebyshev to `from`) that has an access tile, then that stand's
/// access tile nearest `from`. `None` when no packed stand has one.
pub fn nearest_bank_access(
    collision: &WorldCollision,
    stands: &[BankStand],
    from: WorldTile,
) -> Option<WorldTile> {
    stands
        .iter()
        .flat_map(|stand| bank_access_tiles(collision, stand).map(move |tile| (stand, tile)))
        .min_by_key(|(stand, tile)| {
            (
                i32::from(stand.tile.level != from.level),
                (stand.tile.x - from.x)
                    .abs()
                    .max((stand.tile.z - from.z).abs()),
                (tile.x - from.x).abs().max((tile.z - from.z).abs()),
                tile.x,
                tile.z,
            )
        })
        .map(|(_, tile)| tile)
}

/// One step of a [`BankFetch`] session, in execution order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BankStep {
    /// Walk to a standable access tile of the nearest packed stand (the
    /// host routes it). Never the stand's own interact tile.
    Walk { x: i32, z: i32, level: i32 },
    /// Open the bank (booth `Use-quickly` / teller op).
    Open,
    /// Deposit the whole backpack.
    DepositAll,
    /// Withdraw `count` of the obj from the open bank: an `item_req`
    /// count, or 1 for a `worn_req` the session then wears.
    Withdraw { id: i32, count: i32 },
    /// Wear/wield the obj from the inventory (`worn_req`). A bank trip
    /// wears only after [`BankStep::Close`]: the client cannot wear from
    /// the backpack while the bank is open.
    Wear { id: i32 },
    /// Close the bank.
    Close,
}

/// The BankBudget session, planned from a [`crate::router::find_with`]
/// `NoPath` under [`crate::router::FindOptions::allow_bank_fetch`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BankFetch {
    /// The ordered steps the pump executes.
    pub steps: Vec<BankStep>,
    /// The world-state the steps leave behind: the backpack deposited,
    /// every `item_req` stack withdrawn to its needed count, every
    /// `worn_req` obj worn. The post-session strict re-find must pass
    /// against this state; anything less fails closed.
    pub state: WorldState,
}

/// Plan a BankBudget session for a strict [`find_with`] `NoPath` whose
/// diagnosis is `missing` ([`crate::router::find_missing_item_reqs`], or
/// [`crate::router::missing_item_reqs`] of a route found under
/// [`fetchable_state`]).
/// `state` is the search's gating facts; `bank` is the **open** bank's
/// rows (obj id, count) from the live snapshot — empty when the bank is
/// closed (BankBudget has no closed-bank inventory, so a needed item
/// that is only banked cannot be proved and this returns `None`);
/// `stands` is the packed bank stand table
/// ([`crate::world::NavWorld::banks`]), booths and NPC tellers; `from`
/// is the player's tile, which picks the nearest stand that has a
/// standable access neighbour ([`nearest_bank_access`]).
///
/// A `worn_req` alternative already carried plans only [`BankStep::Wear`]
/// — no bank walk. Otherwise the plan walks to that access tile, opens
/// the bank, deposits the backpack, withdraws every missing item, closes,
/// and then wears each `worn_req` one. The deposit supplies the
/// bank with the carried stack, so a `worn_req` alternative that is
/// merely carried is fetchable after it; every needed amount must be
/// covered by bank + carried stacks combined. `None` when the plan
/// cannot be built — the caller reports
/// [`crate::router::RouteError::NoPath`].
pub fn plan_bank_fetch(
    missing: &[MissingReq],
    state: &WorldState,
    bank: &[(i32, i32)],
    stands: &[BankStand],
    from: WorldTile,
    collision: &WorldCollision,
) -> Option<BankFetch> {
    if missing.is_empty() {
        return None;
    }
    // Wear-from-inventory only: every missing req is a worn_req with at
    // least one alternative already carried. No bank trip at all.
    let all_worn_carried = missing.iter().all(|r| match r {
        MissingReq::WearAny { ids } => ids
            .iter()
            .any(|id| state.inv.get(id).is_some_and(|&c| c >= 1)),
        MissingReq::Carry { .. } => false,
    });
    if all_worn_carried {
        let mut post = state.clone();
        let mut steps = Vec::new();
        for r in missing {
            match r {
                MissingReq::WearAny { ids } => {
                    let id = ids
                        .iter()
                        .find(|id| state.inv.get(id).is_some_and(|&c| c >= 1))
                        .copied()
                        .expect("all-worn-carry checked the alternatives");
                    post.worn.insert(id);
                    if let Some(c) = post.inv.get_mut(&id) {
                        *c -= 1;
                        if *c <= 0 {
                            post.inv.remove(&id);
                        }
                    }
                    steps.push(BankStep::Wear { id });
                }
                MissingReq::Carry { .. } => unreachable!("all-worn-carry arm"),
            }
        }
        return Some(BankFetch { steps, state: post });
    }

    // Bank trip. The deposit moves the carried stacks into the bank, so
    // every needed amount may come from the bank's rows plus the
    // backpack: `supply` is the combined count, saturated at a full stack.
    let bank_count = |id: i32| {
        bank.iter()
            .find(|&&(i, _)| i == id)
            .map(|&(_, c)| c)
            .unwrap_or(0)
    };
    let supply = |id: i32| bank_count(id).saturating_add(state.inv.get(&id).copied().unwrap_or(0));
    for r in missing {
        match r {
            MissingReq::Carry { id, count } => {
                if supply(*id) < *count {
                    return None;
                }
            }
            MissingReq::WearAny { ids } => {
                if !ids.iter().any(|id| supply(*id) >= 1) {
                    return None;
                }
            }
        }
    }
    // Walk dest is a standable access tile, never the interact tile.
    let access = nearest_bank_access(collision, stands, from)?;

    let mut steps = vec![
        BankStep::Walk {
            x: access.x,
            z: access.z,
            level: access.level,
        },
        BankStep::Open,
        BankStep::DepositAll,
    ];
    // The deposit clears the backpack; the withdrawals rebuild it to
    // exactly what the strict gate needs. Wearing waits for the close.
    let mut post = state.clone();
    post.inv.clear();
    let mut wear = Vec::new();
    for r in missing {
        match r {
            MissingReq::Carry { id, count } => {
                steps.push(BankStep::Withdraw {
                    id: *id,
                    count: *count,
                });
                post.inv.insert(*id, *count);
            }
            MissingReq::WearAny { ids } => {
                let id = ids
                    .iter()
                    .find(|id| supply(**id) >= 1)
                    .copied()
                    .expect("the supply check passed an alternative");
                steps.push(BankStep::Withdraw { id, count: 1 });
                wear.push(BankStep::Wear { id });
                post.worn.insert(id);
            }
        }
    }
    steps.push(BankStep::Close);
    steps.append(&mut wear);
    Some(BankFetch { steps, state: post })
}

/// The facts a BankBudget session can establish from `state`, whichever
/// route it serves: exactly the supply [`plan_bank_fetch`] checks. Every
/// carried obj can be worn in place; with a bank stand to walk to, every
/// obj in the open bank's rows (an obj's first row, as the planner reads
/// it) plus the backpack can be carried at their combined count (saturated
/// at a full stack, so a bank row never takes a carried fact away), and
/// worn.
/// A strict search under this state reaches only goals whose
/// `item_req`/`worn_req` gates a session can meet, and [`plan_bank_fetch`]
/// plans every such route's missing facts, so a goal behind an obj the
/// session cannot get never hides one it can (frozen `virtualizeWithItems`
/// likewise searches with the bank's objs assumed held).
pub fn fetchable_state(
    state: &WorldState,
    bank: &[(i32, i32)],
    stands: &[BankStand],
) -> WorldState {
    let mut fetchable = state.clone();
    fetchable.worn.extend(
        state
            .inv
            .iter()
            .filter(|&(_, &n)| n >= 1)
            .map(|(&id, _)| id),
    );
    if !stands.is_empty() {
        let mut read = HashSet::new();
        for &(id, count) in bank {
            if read.insert(id) && count >= 1 {
                let held = fetchable.inv.entry(id).or_insert(0);
                *held = held.saturating_add(count);
                fetchable.worn.insert(id);
            }
        }
    }
    fetchable
}

#[cfg(test)]
#[path = "bank_fetch_tests.rs"]
mod tests;
