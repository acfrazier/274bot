//! BankBudget (Task 8): the fetch-and-wear session that unblocks a
//! [`crate::router::find_with`] `NoPath` whose only missing gates are
//! `item_req`, `consumed_req`, or the legacy any-of `worn_req`. `worn_all_req`
//! remains strict in both route searches: a fetched or carried piece cannot
//! stand in for currently equipped gear. `find` stays fail-closed:
//! the router never inserts a virtual bank leg or relaxes an edge; this
//! session is the only thing that may fetch, and the host re-runs the
//! strict search after the steps land.
//!
//! The session is a **plan**: ordered steps the host pump executes through
//! the [`crate::traveller::Traveller`] and `api::interact` bank path (open,
//! withdraw, then Wear or Close), plus the [`WorldState`] those steps leave
//! behind for the post-session re-find. A `worn_req` alternative already
//! carried plans a bare [`BankStep::Wear`] — no bank walk. Anything the plan
//! cannot supply (neither carried nor bankable, or the relaxed diagnosis
//! shows a skill/quest/varp/worn-all gate) is [`BankFetchError::Unavailable`].
//! An unreachable bank retains its strict query failure and zone witness.

use std::borrow::Cow;
use std::collections::HashSet;

use api::bank_memory::{BankMemory, Origin};
use api::snapshot::WorldTile;
use client::dash3d::CollisionFlag;

use crate::collision::WorldCollision;
use crate::named_banks::{is_access_tile, AccessReach, Footprint};
use crate::pack::BankStand;
use crate::router::{
    find_blocking_zones, find_first_with_avoid, find_with_avoid, AvoidRect, FindOptions,
    MissingReq, MissingReqView, MissingRequirements, RouteError,
};
use crate::transport::TransportGraph;
use crate::world_state::WorldState;
use crate::zones::ZoneKey;
/// Borrowed view of route requirements used by the fetch planners.
///
/// The legacy router result owns worn alternatives. A compact diagnosis borrows
/// them from its retained graph lease. This view keeps one planning kernel for both.
pub trait MissingRequirementsView {
    /// Number of requirements in legacy diagnostic order.
    fn len(&self) -> usize;

    /// Borrow one requirement without copying a worn-alternative array.
    fn requirement_at(&self, index: usize) -> Option<MissingReqView<'_>>;

    /// Whether this view contains no requirements.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl MissingRequirementsView for [MissingReq] {
    fn len(&self) -> usize {
        <[MissingReq]>::len(self)
    }

    fn requirement_at(&self, index: usize) -> Option<MissingReqView<'_>> {
        match self.get(index)? {
            MissingReq::Carry { id, count } => Some(MissingReqView::Carry {
                id: *id,
                count: *count,
            }),
            MissingReq::WearAny { ids } => Some(MissingReqView::WearAny { ids }),
        }
    }
}

impl MissingRequirementsView for Vec<MissingReq> {
    fn len(&self) -> usize {
        self.as_slice().len()
    }

    fn requirement_at(&self, index: usize) -> Option<MissingReqView<'_>> {
        self.as_slice().requirement_at(index)
    }
}

impl<const N: usize> MissingRequirementsView for [MissingReq; N] {
    fn len(&self) -> usize {
        N
    }

    fn requirement_at(&self, index: usize) -> Option<MissingReqView<'_>> {
        self.as_slice().requirement_at(index)
    }
}

impl MissingRequirementsView for MissingRequirements {
    fn len(&self) -> usize {
        MissingRequirements::len(self)
    }

    fn requirement_at(&self, index: usize) -> Option<MissingReqView<'_>> {
        MissingRequirements::requirement_at(self, index)
    }
}

mod cursor;
mod overlay;
mod planning;

pub use cursor::{BankFetchCursor, BankFetchPlan};
pub use overlay::{FetchableCursorError, FetchableEdgeCursor, FetchableStateCursor};
pub use planning::PlanningRowsCursor;

/// Packed stands within this Chebyshev of each other belong to one bank
/// building: the Walk arrives at any of their access tiles and Open may use
/// any of their booths or tellers.
pub const SAME_BANK: i32 = 12;

/// An account's bank rows as the host hands them to a planner: the memory's
/// `(obj id, count)` rows with the origin they came from
/// (design-bank-snapshot §1.2, §2.4). `Unknown` with no rows is the
/// never-observed bank. The rows are one copy per walk arm, never per tick;
/// while the bank is open and loaded the memory mirrors the live table, so
/// this is also the open-bank case. A holder that keeps them inline may
/// store the two fields apart; every planner reads them through
/// [`planning_rows`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BankRows {
    pub origin: Origin,
    pub rows: Vec<(i32, i32)>,
}

impl BankRows {
    /// One copy of `memory`'s rows with its origin.
    pub fn of(memory: &BankMemory) -> Self {
        Self {
            origin: memory.origin(),
            rows: memory.rows().to_vec(),
        }
    }
}

/// The rows [`plan_bank_fetch`] plans a session for the route diagnosis
/// `missing` over, from bank rows `rows` of origin `origin`
/// (design-bank-snapshot §2.4, §4 F6). `Session` rows are this session's
/// observation and `Unknown` rows are empty: both are returned as they are
/// (borrowed), so a shortage there is `NoPath`. A `Hint` is advisory, so a
/// shortage it predicts costs one verifying trip instead: each diagnosed
/// `Carry { id, count }` is raised to `max(hinted, count)` and each
/// `WearAny` none of whose ids is hinted gains one unit of its first id.
/// The open bank then settles the trip (a withdraw it cannot serve aborts
/// the session). Only the given diagnosis is overlaid; the rows stay sorted
/// by id. One small owned `Vec` per plan, never per tick.
pub fn planning_rows<'a, M: MissingRequirementsView + ?Sized>(
    origin: Origin,
    rows: &'a [(i32, i32)],
    missing: &M,
) -> Cow<'a, [(i32, i32)]> {
    if origin != Origin::Hint {
        return Cow::Borrowed(rows);
    }
    let mut cursor = PlanningRowsCursor::new();
    while !cursor.advance(rows, missing) {}
    Cow::Owned(cursor.take_rows().expect("the overlay cursor completed"))
}

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
    /// Walk to the reachable access with the cheapest strict path.
    /// Never the stand's own interact tile.
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
    /// Wear/wield the obj from the inventory (`worn_req`). A Wear sent while
    /// the bank is open is accepted by 289, which closes it as it equips.
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
    /// `worn_req` alternative is worn.
    pub state: WorldState,
}

/// Why a synchronous BankBudget plan could not be built.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum BankFetchError {
    /// The requirements cannot be supplied, or no usable bank access exists.
    #[error("bank requirements or access unavailable")]
    Unavailable,
    /// No candidate access was reached. Retain the first rejected query in
    /// the existing air-distance order, including its active-zone witness.
    #[error("bank access {access:?} failed: {error:?}; blocked zones: {blocked_zones:?}")]
    Access {
        /// The first rejected access tile.
        access: WorldTile,
        /// The strict query's failure, without converting budget exhaustion.
        error: RouteError,
        /// Active zone identities witnessed on the unrestricted access route.
        blocked_zones: Vec<ZoneKey>,
    },
}

/// Plan a BankBudget session for a strict [`find_with`] `NoPath` whose
/// diagnosis is `missing` ([`crate::router::find_missing_item_reqs`], or
/// [`crate::router::missing_item_reqs`] of a route found under
/// [`fetchable_state`]).
/// `Carry` counts are the total initial supply the route needs, not the
/// withdrawal amount; when another deficiency exists, they may include
/// targets already held so a matching `WearAny` cannot spend the last
/// carried unit needed by the route.
/// `state` is the search's gating facts; `bank` is the rows the session is
/// planned over — [`planning_rows`] of the account's bank memory
/// (the live table while the bank is open); an item the rows do not cover
/// cannot be proved and this returns [`BankFetchError::Unavailable`];
/// `stands` is the packed bank stand table
/// ([`crate::world::NavWorld::banks`]), booths and NPC tellers; `from`
/// is the player's tile. One strict first-goal search chooses the cheapest
/// reachable access with `state`, `options`, and `avoid`. Equal path costs
/// keep the existing air-distance order.
///
/// A `worn_req` alternative already carried plans only [`BankStep::Wear`]
/// — no bank walk. Otherwise the plan walks to that access tile, opens the
/// bank, withdraws only each Carry or worn-item deficit, and sends Wear for
/// each selected alternative without a separate Close; on 289, Wear closes
/// the bank as it equips. If no Wear is needed, the plan closes after
/// withdrawals. `worn_all_req` is never synthesized or worn by this session.
/// Existing carried inventory is never deposited or cleared. Every required
/// amount must be covered by bank + carried stacks combined. If no access
/// routes, return the first candidate's strict failure and active-zone witness.
/// Diagnose that witness only after the goal-set search fails.
///
/// Cooperative jobs use [`BankFetchCursor`] and move their owned pre-state
/// through [`BankFetchPlan::apply_next`]. This synchronous adapter alone copies
/// the borrowed pre-state after driving that same planning core.
#[expect(
    clippy::too_many_arguments,
    reason = "The bank inputs and the caller's strict route query share one planning adapter."
)]
pub fn plan_bank_fetch<M: MissingRequirementsView + ?Sized>(
    missing: &M,
    state: &WorldState,
    bank: &[(i32, i32)],
    stands: &[BankStand],
    from: WorldTile,
    collision: &WorldCollision,
    graph: &TransportGraph,
    options: FindOptions,
    avoid: &[AvoidRect],
) -> Result<BankFetch, BankFetchError> {
    let mut cursor = BankFetchCursor::new();
    while !cursor.advance(missing, state, bank, stands, from, collision) {}
    let strict = FindOptions {
        allow_bank_fetch: false,
        prefer_target_order: true,
        ..options
    };
    if let Some(first) = cursor.planned_access() {
        let search = find_first_with_avoid(
            collision,
            graph,
            from,
            cursor.access_targets(),
            strict,
            state,
            avoid,
        );
        match search.into_route() {
            Ok(route) => {
                cursor.select_access(route.dest);
            }
            Err(_) => {
                // A goal-set failure need not equal the first candidate's
                // single-target budget outcome. Preserve that exact error.
                let error = find_with_avoid(collision, graph, from, first, strict, state, avoid)
                    .expect_err("the failed goal set contains the first candidate");
                return Err(BankFetchError::Access {
                    access: first,
                    error,
                    blocked_zones: if error == RouteError::NoPath {
                        find_blocking_zones(collision, graph, from, first, strict, state, avoid)
                            .unwrap_or_default()
                    } else {
                        Vec::new()
                    },
                });
            }
        }
    }
    let mut plan = cursor.take_plan().ok_or(BankFetchError::Unavailable)?;
    let mut post = state.clone();
    while !plan.apply_next(&mut post) {}
    Ok(BankFetch {
        steps: plan.into_steps(),
        state: post,
    })
}

/// The facts a BankBudget session can establish from `state`, whichever
/// route it serves: exactly the supply [`plan_bank_fetch`] checks. Every
/// carried obj can be worn in place; with a bank stand to walk to, every
/// obj in `bank` (an obj's first row, as the planner reads it) plus the
/// backpack can be carried at their combined count, and made
/// available to the legacy `worn_req` any-of gate. These are candidate
/// items, not observed equipment, so `worn_all_req` stays exact.
/// A strict search under this state reaches only goals whose `item_req`,
/// `consumed_req`, or `worn_req` gates a session can meet, and
/// [`plan_bank_fetch`] budgets the whole route's missing facts. A goal
/// behind an obj the session cannot get never hides one it can (frozen
/// `virtualizeWithItems` likewise searches with the bank's objs assumed
/// held). `bank` is the memory's rows; for a `Hint` memory the host passes
/// [`planning_rows`] of a goal-set diagnosis
/// ([`crate::router::find_first_missing_item_reqs_with_avoid`]), so a
/// hinted shortage does not hide the one trip that would verify it.
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
