//! BankBudget (Task 8): the fetch-and-wear session that unblocks a
//! [`crate::router::find_with`] `NoPath` whose only missing gates are
//! `item_req`, `consumed_req`, or the legacy any-of `worn_req`. `worn_all_req`
//! remains strict in both route searches: a fetched or carried piece cannot
//! stand in for currently equipped gear. `find` stays fail-closed:
//! the router never inserts a virtual bank leg or relaxes an edge; this
//! session is the only thing that may fetch, and the host re-runs the
//! strict search after the steps land.
//!
//! The session is a **plan**: ordered steps the host pump executes
//! through the [`crate::traveller::Traveller`] and the `api::interact`
//! bank path (open/withdraw/close/wear), plus the [`WorldState`]
//! those steps leave behind for the post-session re-find. A `worn_req`
//! alternative already carried plans a bare [`BankStep::Wear`] — no
//! bank walk. Anything the plan cannot supply (neither carried nor
//! bankable, or the relaxed diagnosis shows a skill/quest/varp/worn-all gate) is
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
    /// Withdraw exactly `count` units from the open bank using a fixed
    /// amount or Withdraw All when it is the whole bank stack.
    Withdraw { id: i32, count: i32 },
    /// Open the bank's Withdraw-X amount dialog for `id`; followed by
    /// [`BankStep::WithdrawXAmount`].
    WithdrawX { id: i32 },
    /// Answer the open Withdraw-X dialog with the exact amount.
    WithdrawXAmount { id: i32, count: i32 },
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
    /// The world-state the steps leave behind: every carried item is
    /// preserved, short Carry stacks are topped up, and each selected
    /// `worn_req` alternative is worn after the bank closes.
    pub state: WorldState,
}

/// Plan a BankBudget session for a strict [`find_with`] `NoPath` whose
/// diagnosis is `missing` ([`crate::router::find_missing_item_reqs`], or
/// [`crate::router::missing_item_reqs`] of a route found under
/// [`fetchable_state`]).
/// `Carry` counts are the total initial supply the route needs, not the
/// withdrawal amount; when another deficiency exists, they may include
/// targets already held so a matching `WearAny` cannot spend the last
/// carried unit needed by the route.
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
/// the bank, withdraws only each Carry or worn-item deficit, closes, and
/// then wears each selected `worn_req` item. `worn_all_req` is never
/// synthesized or worn by this session. Existing carried inventory is
/// never deposited or cleared. Every required amount must be covered
/// by bank + carried stacks combined. `None` when the plan cannot be
/// built — the caller reports [`crate::router::RouteError::NoPath`].
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

    let bank_count = |id: i32| {
        bank.iter()
            .find(|&&(i, _)| i == id)
            .map_or(0, |&(_, count)| count.max(0))
    };
    let held_count = |id: i32| state.inv.get(&id).copied().unwrap_or(0).max(0);

    // Carry rows describe total initial inventory needed along the route.
    // Use one target per obj id so duplicate diagnoses do not plan duplicate
    // withdrawals.
    let mut carry = Vec::<(i32, i32)>::new();
    for req in missing {
        let MissingReq::Carry { id, count } = req else {
            continue;
        };
        if *count <= 0 {
            continue;
        }
        if let Some((_, target)) = carry.iter_mut().find(|(held, _)| held == id) {
            *target = (*target).max(*count);
        } else {
            carry.push((*id, *count));
        }
    }
    let carry_count = |id: i32| {
        carry
            .iter()
            .find(|&&(held, _)| held == id)
            .map_or(0, |&(_, count)| count)
    };

    // Select one available alternative for each missing worn gate. Prefer a
    // carried surplus, but reserve every Carry target first: a carried item
    // that is also the selected wearer must have an extra unit beyond the
    // count the route needs kept in inventory.
    let mut wear = Vec::<i32>::new();
    for req in missing {
        let MissingReq::WearAny { ids } = req else {
            continue;
        };
        if ids
            .iter()
            .any(|id| state.worn.contains(id) || wear.contains(id))
        {
            continue;
        }

        let mut carried_choice = None;
        let mut available_choice = None;
        for &id in ids {
            let reserved = carry_count(id);
            let held = held_count(id);
            let total = held.saturating_add(bank_count(id));
            if held > reserved && carried_choice.is_none() {
                carried_choice = Some(id);
            }
            if total > reserved && available_choice.is_none() {
                available_choice = Some(id);
            }
        }
        let id = carried_choice.or(available_choice)?;
        wear.push(id);
    }

    // The route needs its initial Carry budget plus a separate item for
    // every chosen wearable that shares an obj id with that budget.
    let mut target_ids: Vec<i32> = carry.iter().map(|&(id, _)| id).collect();
    for &id in &wear {
        if !target_ids.contains(&id) {
            target_ids.push(id);
        }
    }
    let mut withdrawals = Vec::<(i32, i32)>::new();
    for id in target_ids {
        let target = carry_count(id).checked_add(i32::from(wear.contains(&id)))?;
        let held = held_count(id);
        if held.saturating_add(bank_count(id)) < target {
            return None;
        }
        let count = target.saturating_sub(held);
        if count > 0 {
            withdrawals.push((id, count));
        }
    }

    // When everything is already carried, wear in place and avoid a bank
    // trip. A bank trip is needed only when there is a real shortage.
    let needs_bank = !withdrawals.is_empty();
    let mut steps = if needs_bank {
        let access = nearest_bank_access(collision, stands, from)?;
        vec![
            BankStep::Walk {
                x: access.x,
                z: access.z,
                level: access.level,
            },
            BankStep::Open,
        ]
    } else if wear.is_empty() {
        return None;
    } else {
        Vec::new()
    };

    if needs_bank {
        for &(id, count) in &withdrawals {
            if matches!(count, 1 | 5 | 10) || count == bank_count(id) {
                steps.push(BankStep::Withdraw { id, count });
            } else {
                steps.push(BankStep::WithdrawX { id });
                steps.push(BankStep::WithdrawXAmount { id, count });
            }
        }
        steps.push(BankStep::Close);
    }
    for &id in &wear {
        steps.push(BankStep::Wear { id });
    }

    // No step clears the backpack: retain its full snapshot and add only
    // the calculated shortages before applying the selected wear moves.
    let mut post = state.clone();
    for &(id, count) in &withdrawals {
        let held = post.inv.entry(id).or_insert(0);
        *held = held.saturating_add(count);
    }
    for id in wear {
        post.worn.insert(id);
        let exhausted = if let Some(held) = post.inv.get_mut(&id) {
            *held -= 1;
            *held <= 0
        } else {
            false
        };
        if exhausted {
            post.inv.remove(&id);
        }
    }
    Some(BankFetch { steps, state: post })
}

/// The facts a BankBudget session can establish from `state`, whichever
/// route it serves: exactly the supply [`plan_bank_fetch`] checks. Every
/// carried obj can be worn in place; with a bank stand to walk to, every
/// obj in the open bank's rows (an obj's first row, as the planner reads
/// it) plus the backpack can be carried at their combined count, and made
/// available to the legacy `worn_req` any-of gate. These are candidate
/// items, not observed equipment, so `worn_all_req` stays exact.
/// A strict search under this state reaches only goals whose `item_req`,
/// `consumed_req`, or `worn_req` gates a session can meet, and
/// [`plan_bank_fetch`] budgets the whole route's missing facts. A goal
/// behind an obj the session cannot get never hides one it can (frozen
/// `virtualizeWithItems` likewise searches with the bank's objs assumed
/// held).
pub fn fetchable_state(
    state: &WorldState,
    bank: &[(i32, i32)],
    stands: &[BankStand],
) -> WorldState {
    let mut fetchable = state.clone();
    fetchable.allow_fetchable_worn = true;
    if !stands.is_empty() {
        let mut read = HashSet::new();
        for &(id, count) in bank {
            if read.insert(id) && count >= 1 {
                let held = fetchable.inv.entry(id).or_insert(0);
                *held = held.saturating_add(count);
            }
        }
    }
    fetchable
}

#[cfg(test)]
#[path = "bank_fetch_tests.rs"]
mod tests;
