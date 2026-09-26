//! Dijkstra routing over the whole-world collision bake (`WorldCollision`)
//! and the content-derived transport graph (`TransportGraph`), plus the
//! legacy step-grid A* kept under [`find_on_grid`] for the traveller and
//! live harnesses until the collision+transport router replaces them.
//!
//! One Dijkstra minimizes **total ticks**: a walk step costs the search's
//! run rate ([`CostModel::running`]: 0.5 ticks/tile, 2 tiles/tick) and a
//! transport edge costs its `ticks` (OP_BASE + duration). Costs are `f64`
//! — 0.5 and every reachable sum are exact — ordered with `total_cmp` in
//! the heap so equal costs tie-break on tile coordinates. A step into a
//! neighbour is allowed only when the neighbour's derived walkable word
//! passes the client's directional movement test (the `PL_WALK_*` masks in
//! `tryMove`), never the blanket `walkable()` check.
//!
//! [`find`] and [`find_with_model`] never see teleports; the any-tile
//! teleport layer ([`TransportGraph::teleports`]) only joins the search
//! through [`find_allow_teleports`]/[`find_allow_teleports_with_model`].
//! Wilderness tiles ([`TransportGraph::wilderness`]) are refused unless
//! the search's [`FindOptions::allow_wilderness`] is set; every option'd
//! entry point is [`find_with`]. A graph with no packed zones (a legacy
//! 274N grid) gates nothing. Every transport edge — walked or teleported
//! — is additionally gated by the search's [`WorldState`]: an edge whose
//! requirements the state cannot prove is never relaxed (missing facts
//! fail closed).

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};
use std::time::Instant;

use api::snapshot::WorldTile;
use client::dash3d::CollisionFlag;

use crate::collision::WorldCollision;
use crate::essence::{EssenceSession, ESSENCE_MINE_EXIT_TICKS, ESSENCE_MINE_PORTALS};
use crate::transport::{TransportEdge, TransportGraph};

use crate::world_state::WorldState;

/// One leg of a route: a walk run or one transport crossing. Consecutive
/// walk tiles collapse into a single `Walk` leg; each transport edge is
/// its own `Transport` leg.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Leg {
    Walk { tiles: Vec<WorldTile> },
    Transport { edge: TransportEdge },
}

/// A route from the `find` origin to `dest`. `ticks` is the total tick
/// cost of the whole route: walk steps at the search's run rate (0.5 per
/// tile, exact in f64) plus every transport edge's `ticks`. Half-ticks are
/// never truncated.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    pub legs: Vec<Leg>,
    pub dest: WorldTile,
    pub ticks: f64,
}

/// Run pace: 2 tiles per tick, 0.5 ticks per tile.
pub const PER_STEP_RUN: f64 = 0.5;
/// Walk pace: 1 tile per tick, 1 tick per tile.
pub const PER_STEP_WALK: f64 = 1.0;

/// The per-search walking rate. This task's searches walk at the fixed
/// running pace ([`CostModel::running`]); the agility/regen energy-aware
/// fallback will flip to the walk pace when run energy can't sustain
/// running.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CostModel {
    /// Ticks per walked tile at the running pace (0.5).
    pub run_per_step: f64,
    /// Ticks per walked tile at the walking pace (1.0).
    pub walk_per_step: f64,
}

impl CostModel {
    /// The fixed running rate this task searches with.
    pub fn running() -> Self {
        CostModel {
            run_per_step: PER_STEP_RUN,
            walk_per_step: PER_STEP_WALK,
        }
    }
}

/// Per-search opt-ins, all default off so [`find`] keeps the safe
/// defaults. `allow_teleports` unions the any-tile teleport layer in;
/// `allow_wilderness` lets the search step into (or land in) packed
/// [`TransportGraph::wilderness`] zones. A graph with no packed zones
/// gates nothing.
/// `allow_bank_fetch` is the BankBudget opt-in: on its own it never
/// inserts a bank leg or relaxes an item req — an edge stays unusable
/// unless the search's [`WorldState`] already proves it. The
/// fetch-and-wear session lives outside the router
/// ([`crate::bank_fetch::plan_bank_fetch`], whose diagnosis arm is
/// [`find_missing_item_reqs`]); a caller that sets the flag but plans no
/// session gets exactly the fail-closed search. `essence` is the
/// per-slot Rune Essence mine latch
/// ([`EssenceSession`]): when the player stands inside the enclosed mine
/// the search relaxes the exit portal's return hop to the entry wizard's
/// overworld anchor. `None` (the default) keeps the mine a sealed dead
/// end — the pack carries no return edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FindOptions {
    pub allow_teleports: bool,
    pub allow_wilderness: bool,
    pub allow_bank_fetch: bool,
    pub essence: Option<EssenceSession>,
}

/// Axis-aligned avoidance rectangle for inspect-local routing. Bounds are
/// inclusive on both axes. When [`AvoidRect::level`] is [`None`], the rect
/// applies on every plane; otherwise only that level matches.
///
/// Callers must supply valid geometry (`min_x <= max_x`, `min_z <= max_z`).
/// Invalid rects are not validated here and do not produce a router error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AvoidRect {
    pub min_x: i32,
    pub max_x: i32,
    pub min_z: i32,
    pub max_z: i32,
    /// `None` = every level.
    pub level: Option<i32>,
}

impl AvoidRect {
    /// Whether `tile` sits inside this rect (inclusive bounds, optional level).
    pub fn contains(self, tile: WorldTile) -> bool {
        if let Some(lvl) = self.level {
            if tile.level != lvl {
                return false;
            }
        }
        tile.x >= self.min_x && tile.x <= self.max_x && tile.z >= self.min_z && tile.z <= self.max_z
    }
}

/// Why [`find`] failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteError {
    /// No walk/transport path reaches the destination.
    NoPath,
    /// The node-expansion budget was exhausted before reaching it.
    BudgetExhausted,
}

/// Per-target failure of a shared search. Unlike single-target
/// [`RouteError`], a caller deadline can leave a target unsettled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetError {
    NoPath,
    BudgetExhausted,
    NotSettled,
}

impl From<RouteError> for TargetError {
    fn from(error: RouteError) -> Self {
        match error {
            RouteError::NoPath => Self::NoPath,
            RouteError::BudgetExhausted => Self::BudgetExhausted,
        }
    }
}

/// Node-expansion budget bounding a [`find`] search — the m8aq route-cutoff
/// concept. The whole 2004 world is ~16M tiles, so any real route stays far
/// under this; it only stops pathological floods.
pub const NODE_BUDGET: usize = 4_000_000;

/// The interact radius (chebyshev) a transport edge is usable from: any
/// standable tile within this distance of the edge's `at` is a valid
/// take-off. The game only accepts `op_loc` from adjacent (1). A radius
/// of 3 let Dijkstra "use" a door through a fence (Lumbridge cow pen:
/// inside tile to the north-west road gate) and the walker then walked
/// into the wall. The approach is derived at expansion time, never baked.
const INTERACT_RADIUS: i32 = 1;

/// The eight step deltas (client coordinates: +x east, +z north).
const STEPS: [(i32, i32); 8] = [
    (0, 1),
    (0, -1),
    (1, 0),
    (-1, 0),
    (-1, -1),
    (1, -1),
    (-1, 1),
    (1, 1),
];

/// The client's `PL_WALK_*` movement masks, as `u32` to match the flag word.
const MASK_N: u32 = CollisionFlag::PL_WALK_N as u32;
const MASK_E: u32 = CollisionFlag::PL_WALK_E as u32;
const MASK_S: u32 = CollisionFlag::PL_WALK_S as u32;
const MASK_W: u32 = CollisionFlag::PL_WALK_W as u32;
const MASK_NE: u32 = CollisionFlag::PL_WALK_NE as u32;
const MASK_SE: u32 = CollisionFlag::PL_WALK_SE as u32;
const MASK_NW: u32 = CollisionFlag::PL_WALK_NW as u32;
const MASK_SW: u32 = CollisionFlag::PL_WALK_SW as u32;

/// Dijkstra over `collision` walk steps (each costing the search's run
/// rate) and `graph` transport edges (each costing `edge.ticks`), all in
/// one heap minimizing total ticks. Walk steps are 8-directional and stay
/// on `from`'s level; transports may change level.
///
/// The origin may sit on a blocked tile (a loc-blocked tele landing); only
/// the tiles stepped *onto* are tested. Destinations are reached exactly:
/// a blocked destination is `NoPath` unless a transport lands on it.
/// Edges are gated by the fail-closed empty [`WorldState`] — an edge
/// whose requirements nothing proves is never relaxed. Callers that know
/// the player's facts use [`find_with`].
pub fn find(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
) -> Result<Route, RouteError> {
    find_with(
        collision,
        graph,
        from,
        to,
        FindOptions::default(),
        &WorldState::empty(),
    )
}

/// [`find`] with explicit opt-ins ([`FindOptions::allow_teleports`],
/// [`FindOptions::allow_wilderness`], and [`FindOptions::allow_bank_fetch`])
/// and the gating [`WorldState`]: an edge is relaxed only when
/// every `skill_req` / `item_req` / `quest_req` / `varp_req` / `worn_req`
/// is satisfied by the state. Missing facts fail closed — the flag alone
/// never fetches ([`find_missing_item_reqs`] is the session's diagnosis
/// arm, and the session itself lives in [`crate::bank_fetch`]).
pub fn find_with(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
    opts: FindOptions,
    state: &WorldState,
) -> Result<Route, RouteError> {
    find_with_avoid(collision, graph, from, to, opts, state, &[])
}

/// [`find_with`] with inspect-local avoidance rectangles. Walk steps never
/// enter a rect from outside; a start already inside any rect stays
/// searchable until the route leaves the union. Transport, any-tile
/// teleport, and essence-return landings follow the same outside-in rule;
/// takeoff `at` tiles are not independently filtered.
pub fn find_with_avoid(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
    opts: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
) -> Result<Route, RouteError> {
    find_with_avoid_bounded(collision, graph, from, to, opts, state, avoid, NODE_BUDGET)
}

/// [`find_with_avoid`] with an injectable node-expansion cap. Production
/// inspect uses [`find_with_avoid`] (the default [`NODE_BUDGET`]); tests
/// may pass a smaller bound to distinguish `BudgetExhausted` from `NoPath`.
#[allow(clippy::too_many_arguments)] // collision/graph/tiles/opts/state/avoid/budget surface
pub fn find_with_avoid_bounded(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
    opts: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
    budget: usize,
) -> Result<Route, RouteError> {
    find_bounded_impl(
        collision,
        graph,
        from,
        to,
        CostModel::running(),
        budget,
        opts.allow_teleports,
        opts.allow_wilderness,
        state,
        opts.essence.as_ref(),
        avoid,
    )
}

/// Native bank-search cap: 500,000 non-goal expansions precede an accepted
/// goal, which has 1-based settle ordinal 500,001.
pub const BANK_TARGET_BUDGET: usize = 500_001;
/// Settles a first-goal search's goal set keeps the search going while the
/// set's backward proof has not shown any of its goals reachable. A proof of
/// reachability lifts the set to [`find_with`]'s full budget; a proof of
/// unreachability stops it at once.
/// Only a goal set unreachable from a large region whose own backward region
/// is also large spends all of it: on the 289 bake, stands fed through
/// ladders by the unstamped upper planes. Measured reachable in-scene stands
/// need at most ~347,000 settles (a 598-tick Wilderness detour; ~279,000
/// with every teleport usable), and 2^19 settles hold about 90 MB of search
/// tables.
pub const FIRST_TARGET_BUDGET: usize = 1 << 19;
/// Tiles the backward proof may admit before it leaves the answer to the
/// forward search alone. It takes one step per settled node, so this only
/// caps its own set (a 12-byte tile each, plus hash-table overhead) when
/// the forward budget is larger.
const REVERSE_PROOF_BUDGET: usize = 1 << 18;

/// A target's exact shortest-path cost and 1-based shared settle ordinal.
/// An origin shortcut has ordinal zero.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TargetCost {
    pub ticks: f64,
    pub settled_at: usize,
}

/// Allocated entry capacities for this request's search scratch at stop.
/// HashMap/heap capacities never shrink during the flood; these are their
/// peak allocated entry capacities, not retained per worker or pack bytes.
/// `reverse` and `reverse_queue` are the larger of the goal sets' backward
/// proof sets and queues (zero when no proof ran); each proof's scratch is
/// dropped as soon as it decides or its set stops.
#[derive(Debug, Clone, Copy, Default)]
pub struct SearchCapacities {
    pub distances: usize,
    pub predecessors: usize,
    pub settled: usize,
    pub heap: usize,
    pub reverse: usize,
    pub reverse_queue: usize,
}

/// How the backward proof beside a goal search ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReverseProof {
    /// No proof ran: an empty goal set, an origin shortcut, or a search
    /// kind that does not carry one.
    #[default]
    NotRun,
    /// Undecided when the forward search settled a goal or hit its budget.
    Open,
    /// The backward region met the origin, or the landing of a teleport the
    /// origin can take: a goal is reachable, and the forward search keeps
    /// its full budget to find the cheapest.
    Reachable,
    /// A teleport usable elsewhere lands in the backward region (every tile
    /// may then precede it), or the region outgrew the proof's cap; the
    /// forward search decides within its budget.
    Abandoned,
    /// The backward region closed without the origin: no goal is reachable.
    Unreachable,
}

/// What a first-goal search established about its fallback goal set.
#[derive(Debug, Clone, PartialEq)]
pub enum FallbackRoute {
    /// The cheapest fallback goal, settled on the way.
    Routed(Route),
    /// No fallback goal can settle: the set's own proof shows it
    /// unreachable, or everything reachable settled
    /// ([`RouteError::NoPath`]); or a settle past its own budget exists
    /// without its proof showing a goal reachable
    /// ([`RouteError::BudgetExhausted`]).
    Failed(RouteError),
    /// Only more settles could decide it: its proof shows a goal reachable,
    /// or the preferred proof stopped the search within the fallback's
    /// budget. A search over the fallback set alone decides it.
    Undecided,
}

/// One first-settled-target search. Dijkstra settles nodes in increasing
/// route cost, so the route is the cheapest reachable input target without
/// paying to prove whether every other target is reachable. A search with a
/// fallback goal set ([`find_first_with_fallback`]) also reports what it
/// established about that set when no preferred goal routes.
pub struct FirstRouteSearch {
    route: Result<Route, RouteError>,
    fallback: Option<FallbackRoute>,
    settled: usize,
    capacities: SearchCapacities,
    proof: ReverseProof,
}

impl FirstRouteSearch {
    pub fn route(&self) -> Result<&Route, RouteError> {
        self.route.as_ref().map_err(|error| *error)
    }

    /// The fallback set's answer, when no preferred goal routed and the
    /// search had a fallback set; `None` otherwise.
    pub fn fallback(&self) -> Option<&FallbackRoute> {
        self.fallback.as_ref()
    }

    pub fn into_route(self) -> Result<Route, RouteError> {
        self.route
    }

    /// The preferred route and, as for [`Self::fallback`], the fallback's.
    pub fn into_routes(self) -> (Result<Route, RouteError>, Option<FallbackRoute>) {
        (self.route, self.fallback)
    }

    /// Forward nodes settled (the backward proof's tiles are not counted).
    pub fn settled(&self) -> usize {
        self.settled
    }

    /// Peak scratch entry capacities at the point the search stopped.
    pub fn scratch_capacities(&self) -> SearchCapacities {
        self.capacities
    }

    /// The preferred goals' backward proof.
    pub fn proof(&self) -> ReverseProof {
        self.proof
    }
}

/// Search until the cheapest reachable target settles, under the same
/// native gates as [`find_with`]. Dijkstra alone reports an unreachable goal
/// set only after flooding everything reachable from `from`, so a backward
/// closure from the targets ([`ReverseProof`]) runs one step per settled
/// node: a goal set sealed in a small region (a dead-end pocket, a gated
/// room) is proven unreachable after about twice that region's size. The
/// search spends at most [`FIRST_TARGET_BUDGET`] settles unless the closure
/// proves a goal reachable, and then keeps [`find_with`]'s full budget.
/// Small goal sets are scanned inline without allocation; larger fallback
/// sets build one membership map instead of scanning every target for every
/// settled node.
pub fn find_first_with(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    targets: &[WorldTile],
    opts: FindOptions,
    state: &WorldState,
) -> FirstRouteSearch {
    find_first_with_fallback(collision, graph, from, targets, &[], opts, state)
}

/// [`find_first_with`] over preferred `targets` with a `fallback` goal set,
/// sharing one search: the cheapest reachable preferred goal wins even over
/// a cheaper fallback goal, and the search records the first fallback goal
/// it settles on the way ([`FallbackRoute`]). The search stops as it would
/// over the preferred goals alone; the fallback set keeps its own budget
/// and backward proof, and whatever it reports is what a search over the
/// fallback alone reports, as far as the shared settles decide it. Its
/// proof starts only when one of its goals settles past its unproven budget
/// or the preferred set stops without a goal, caught up to one step per
/// settle spent (so a fallback goal within that budget, or a preferred goal,
/// costs it nothing, and only one proof's scratch is live at a time). A
/// goal settling past the budget counts only if the proof showed a goal
/// reachable within it; proven unreachable, or past its budget without
/// proving a goal reachable, the set has failed; otherwise it is
/// [`FallbackRoute::Undecided`] for the caller to search alone. Of the two
/// searches, at most one then stops at the unproven budget.
pub fn find_first_with_fallback(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    targets: &[WorldTile],
    fallback: &[WorldTile],
    opts: FindOptions,
    state: &WorldState,
) -> FirstRouteSearch {
    first_search(
        collision,
        graph,
        from,
        targets,
        fallback,
        opts,
        state,
        false,
        &[],
        FIRST_TARGET_BUDGET,
        NODE_BUDGET,
    )
}

/// The proof-carrying goal search behind [`find_first_with_fallback`]
/// (strict gates) and the single-target BankBudget diagnosis
/// (`relax_carry_worn`). The backward closures honor the same gates as the
/// forward search, so a strict proof never hides a relaxed route. Each goal
/// set's `budget` holds until its own proof shows it reachable, then
/// `reachable_budget` applies ([`FirstGoals`]).
#[allow(clippy::too_many_arguments)] // search surface plus relaxation/avoid/budgets
fn first_search(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    targets: &[WorldTile],
    fallback: &[WorldTile],
    opts: FindOptions,
    state: &WorldState,
    relax_carry_worn: bool,
    avoid: &[AvoidRect],
    budget: usize,
    reachable_budget: usize,
) -> FirstRouteSearch {
    if targets.is_empty() {
        return FirstRouteSearch {
            route: Err(RouteError::NoPath),
            fallback: (!fallback.is_empty()).then_some(FallbackRoute::Undecided),
            settled: 0,
            capacities: SearchCapacities::default(),
            proof: ReverseProof::NotRun,
        };
    }
    if targets.contains(&from) {
        return FirstRouteSearch {
            route: Ok(Route {
                legs: vec![Leg::Walk { tiles: vec![from] }],
                dest: from,
                ticks: 0.0,
            }),
            fallback: None,
            settled: 0,
            capacities: SearchCapacities::default(),
            proof: ReverseProof::NotRun,
        };
    }

    let gates = ProofGates {
        collision,
        graph,
        state,
        essence: opts.essence.as_ref(),
        from,
        use_teleports: opts.allow_teleports,
        allow_wilderness: opts.allow_wilderness,
        relax_carry_worn,
    };
    let mut goals = Goals::First(Box::new(FirstGoals::new(
        gates,
        targets,
        fallback,
        budget,
        reachable_budget,
    )));
    // The goal sets hold the budgets.
    let search = search_kernel(
        collision,
        graph,
        from,
        CostModel::running(),
        usize::MAX,
        opts.allow_teleports,
        opts.allow_wilderness,
        state,
        opts.essence.as_ref(),
        relax_carry_worn,
        avoid,
        &mut goals,
        None,
    );
    let Goals::First(first) = goals else {
        unreachable!("a first-goal search keeps its goals");
    };
    let route_to = |(dest, cost): (WorldTile, TargetCost)| {
        let (legs, ticks) = reconstruct(
            dest,
            &search.came_from,
            graph,
            CostModel::running(),
            opts.essence.as_ref(),
        );
        debug_assert_eq!(ticks, cost.ticks);
        Route { legs, dest, ticks }
    };
    let route = match first.preferred.found {
        Some(goal) => Ok(route_to(goal)),
        None => Err(first.preferred.closed.unwrap_or(RouteError::NoPath)),
    };
    let fallback_route = if route.is_ok() || fallback.is_empty() {
        None
    } else {
        Some(match (first.fallback.found, first.fallback.closed) {
            (Some(goal), _) => FallbackRoute::Routed(route_to(goal)),
            (None, Some(error)) => FallbackRoute::Failed(error),
            (None, None) => FallbackRoute::Undecided,
        })
    };
    FirstRouteSearch {
        route,
        fallback: fallback_route,
        settled: search.settled,
        capacities: search.capacities,
        proof: search.proof,
    }
}

/// Per-input results in caller order, with one predecessor tree for lazy
/// reconstruction. The graph is borrowed, not copied, and the heap/dist/done
/// scratch is dropped when the search returns.
pub struct RoutesToTargets<'a> {
    targets: &'a [WorldTile],
    results: Vec<Result<TargetCost, TargetError>>,
    came_from: HashMap<WorldTile, Back>,
    graph: &'a TransportGraph,
    essence: Option<EssenceSession>,
    settled: usize,
    complete: bool,
    capacities: SearchCapacities,
}

impl RoutesToTargets<'_> {
    pub fn results(&self) -> &[Result<TargetCost, TargetError>] {
        &self.results
    }

    pub fn settled(&self) -> usize {
        self.settled
    }

    /// False only if the caller's deadline interrupted the all-target flood.
    pub fn complete(&self) -> bool {
        self.complete
    }

    /// Per-request peak scratch entry capacities (the scratch is already
    /// dropped except for the predecessor tree needed by [`Self::route`]).
    pub fn scratch_capacities(&self) -> SearchCapacities {
        self.capacities
    }

    /// Reconstruct only the chosen target, without cloning paths for the rest.
    pub fn route(&self, target_index: usize) -> Result<Route, TargetError> {
        let cost = self.results[target_index]?;
        let dest = self.targets[target_index];
        let (legs, ticks) = reconstruct(
            dest,
            &self.came_from,
            self.graph,
            CostModel::running(),
            self.essence.as_ref(),
        );
        debug_assert_eq!(ticks, cost.ticks);
        Ok(Route { legs, dest, ticks })
    }
}

/// Search all targets with the same native options and 4M cap as [`find_with`].
pub fn find_many_with<'a>(
    collision: &WorldCollision,
    graph: &'a TransportGraph,
    from: WorldTile,
    targets: &'a [WorldTile],
    opts: FindOptions,
    state: &WorldState,
) -> RoutesToTargets<'a> {
    find_many_with_avoid_bounded(
        collision,
        graph,
        from,
        targets,
        opts,
        state,
        &[],
        NODE_BUDGET,
    )
}

/// Shared bounded search. Duplicate targets retain separate input rows.
#[allow(clippy::too_many_arguments)]
pub fn find_many_with_avoid_bounded<'a>(
    collision: &WorldCollision,
    graph: &'a TransportGraph,
    from: WorldTile,
    targets: &'a [WorldTile],
    opts: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
    budget: usize,
) -> RoutesToTargets<'a> {
    find_many_with_avoid_bounded_until(
        collision, graph, from, targets, opts, state, avoid, budget, None,
    )
}

/// As above, with a caller-supplied completion deadline. The wall-clock
/// bound is caller policy; the search checks it every 256 heap pops and at
/// exit, reports unsettled targets, and leaves fallback choice to the caller.
#[allow(clippy::too_many_arguments)]
pub fn find_many_with_avoid_bounded_until<'a>(
    collision: &WorldCollision,
    graph: &'a TransportGraph,
    from: WorldTile,
    targets: &'a [WorldTile],
    opts: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
    budget: usize,
    deadline: Option<Instant>,
) -> RoutesToTargets<'a> {
    let mut unique = HashMap::with_capacity(targets.len());
    let mut input_indices = Vec::with_capacity(targets.len());
    let mut costs = Vec::new();
    for &target in targets {
        let index = *unique.entry(target).or_insert_with(|| {
            costs.push(if target == from {
                Ok(TargetCost {
                    ticks: 0.0,
                    settled_at: 0,
                })
            } else {
                Err(TargetError::NotSettled)
            });
            costs.len() - 1
        });
        input_indices.push(index);
    }
    let remaining = costs.iter().filter(|result| result.is_err()).count();
    let mut goals = Goals::Many {
        unique: &unique,
        costs: &mut costs,
        remaining,
    };
    let search = if remaining == 0 {
        SearchOutcome::empty()
    } else {
        search_kernel(
            collision,
            graph,
            from,
            CostModel::running(),
            budget,
            opts.allow_teleports,
            opts.allow_wilderness,
            state,
            opts.essence.as_ref(),
            false,
            avoid,
            &mut goals,
            deadline,
        )
    };
    let error = match search.stop {
        SearchStop::Exhausted => TargetError::NoPath,
        SearchStop::Budget => TargetError::BudgetExhausted,
        SearchStop::Deadline | SearchStop::Completed => TargetError::NotSettled,
    };
    for result in &mut costs {
        if result.is_err() {
            *result = Err(error);
        }
    }
    RoutesToTargets {
        targets,
        results: input_indices
            .into_iter()
            .map(|index| costs[index])
            .collect(),
        came_from: search.came_from,
        graph,
        essence: opts.essence,
        settled: search.settled,
        capacities: search.capacities,
        complete: search.stop != SearchStop::Deadline,
    }
}

/// A missing `item_req`/`worn_req` fact the BankBudget session must
/// supply before a strict [`find_with`] can route: an `item_req` stack
/// count the state cannot prove, or a `worn_req` list (any-of) with no
/// worn alternative. [`missing_item_reqs`] names them for a route;
/// [`find`]/[`find_with`] never relax an edge.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MissingReq {
    /// The edge needs `count` of obj `id` carried (`item_req`).
    Carry { id: i32, count: i32 },
    /// The edge needs any one of `ids` worn (`worn_req` is any-of): the
    /// session fetches whichever alternative the player can obtain.
    WearAny { ids: Vec<i32> },
}

/// Diagnose a strict [`find_with`] `NoPath`: run the same search with
/// only the `item_req`/`worn_req` gates ignored, and collect every such
/// fact on the relaxed route that `state` could not prove. Returns
/// `None` when the relaxed search also fails — a skill/quest/varp gate
/// or a plain hole in the graph blocks, and no fetch-and-wear session
/// can help. This is the BankBudget session's diagnosis arm
/// ([`crate::bank_fetch::plan_bank_fetch`]); [`find`] and [`find_with`]
/// themselves never ignore an item gate — missing facts still fail
/// closed. The relaxed search carries the backward [`ReverseProof`], so an
/// unreachable diagnosis target (a solid tile no transport lands on, a
/// sealed pocket) costs about its own backward region instead of a relaxed
/// flood of everything reachable from `from`.
pub fn find_missing_item_reqs(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
    opts: FindOptions,
    state: &WorldState,
) -> Option<Vec<MissingReq>> {
    find_missing_item_reqs_with_avoid(collision, graph, from, to, opts, state, &[])
}

/// [`find_missing_item_reqs`] with the same avoidance semantics as
/// [`find_with_avoid`].
pub fn find_missing_item_reqs_with_avoid(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
    opts: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
) -> Option<Vec<MissingReq>> {
    find_missing_item_reqs_with_avoid_bounded(
        collision,
        graph,
        from,
        to,
        opts,
        state,
        avoid,
        NODE_BUDGET,
    )
}

/// [`find_missing_item_reqs_with_avoid`] with an injectable node-expansion
/// cap. Production inspect uses the default-budget form.
#[allow(clippy::too_many_arguments)] // collision/graph/tiles/opts/state/avoid/budget surface
pub fn find_missing_item_reqs_with_avoid_bounded(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
    opts: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
    budget: usize,
) -> Option<Vec<MissingReq>> {
    let route = first_search(
        collision,
        graph,
        from,
        std::slice::from_ref(&to),
        &[],
        opts,
        state,
        true,
        avoid,
        budget,
        budget,
    )
    .into_route()
    .ok()?;
    Some(missing_item_reqs(&route, state))
}

/// Every `item_req`/`worn_req` fact on `route` that `state` cannot prove,
/// sorted and deduplicated: what a BankBudget session must supply before
/// `state` allows the route ([`crate::bank_fetch::plan_bank_fetch`]).
pub fn missing_item_reqs(route: &Route, state: &WorldState) -> Vec<MissingReq> {
    let mut missing = Vec::new();
    for leg in &route.legs {
        let Leg::Transport { edge } = leg else {
            continue;
        };
        for &(id, count) in &edge.item_req {
            if state.inv.get(&id).is_none_or(|&c| c < count) {
                missing.push(MissingReq::Carry { id, count });
            }
        }
        // `worn_req` is any-of: nothing is missing while any listed id is
        // worn; with none worn, the session must fetch one alternative.
        // An empty list is no gate at all (matches `WorldState::allows`).
        if !edge.worn_req.is_empty() && !edge.worn_req.iter().any(|id| state.worn.contains(id)) {
            missing.push(MissingReq::WearAny {
                ids: edge.worn_req.clone(),
            });
        }
    }
    missing.sort_by_key(|r| match r {
        MissingReq::Carry { id, .. } => (*id, 0),
        MissingReq::WearAny { ids } => (ids.first().copied().unwrap_or(0), 1),
    });
    missing.dedup();
    missing
}

/// [`find`] with an explicit per-search cost model (the run-vs-walk rate).
pub fn find_with_model(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
    model: CostModel,
) -> Result<Route, RouteError> {
    find_bounded_impl(
        collision,
        graph,
        from,
        to,
        model,
        NODE_BUDGET,
        false,
        false,
        &WorldState::empty(),
        None,
        &[],
    )
}

/// [`find`] with the any-tile teleport layer unioned in: every edge in
/// `graph.teleports` is usable from **any** node at cost `edge.ticks`, so
/// a teleport can take the route across a wall, a level boundary, or half
/// the map. Requirements stay on the edges and are gated by `state` like
/// every other transport edge. Default [`find`]/[`find_with_model`] never
/// see teleports. Wilderness stays refused (`allow_wilderness` off).
pub fn find_allow_teleports(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
    state: &WorldState,
) -> Result<Route, RouteError> {
    find_with(
        collision,
        graph,
        from,
        to,
        FindOptions {
            allow_teleports: true,
            allow_wilderness: false,
            allow_bank_fetch: false,
            ..FindOptions::default()
        },
        state,
    )
}

/// [`find_allow_teleports`] with an explicit per-search cost model.
pub fn find_allow_teleports_with_model(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
    model: CostModel,
    state: &WorldState,
) -> Result<Route, RouteError> {
    find_bounded_impl(
        collision,
        graph,
        from,
        to,
        model,
        NODE_BUDGET,
        true,
        false,
        state,
        None,
        &[],
    )
}

/// [`find`] with an explicit cost model and node-expansion budget (the
/// search gives up with [`RouteError::BudgetExhausted`] once `budget`
/// tiles are settled). Teleports, wilderness, and the gating state stay
/// the [`find`] defaults (empty). Test-only: the budget knob has no
/// production caller yet.
#[cfg(test)]
fn find_bounded(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
    model: CostModel,
    budget: usize,
) -> Result<Route, RouteError> {
    find_bounded_impl(
        collision,
        graph,
        from,
        to,
        model,
        budget,
        false,
        false,
        &WorldState::empty(),
        None,
        &[],
    )
}

/// True when `tile` sits in any avoidance rect.
fn tile_in_any_avoid(tile: WorldTile, avoid: &[AvoidRect]) -> bool {
    avoid.iter().any(|r| r.contains(tile))
}

enum FirstTargets<'a> {
    Inline(&'a [WorldTile]),
    Indexed(HashSet<WorldTile>),
}

impl<'a> FirstTargets<'a> {
    fn new(targets: &'a [WorldTile]) -> Self {
        if targets.len() <= 8 {
            Self::Inline(targets)
        } else {
            Self::Indexed(targets.iter().copied().collect())
        }
    }

    fn contains(&self, tile: &WorldTile) -> bool {
        match self {
            Self::Inline(targets) => targets.contains(tile),
            Self::Indexed(targets) => targets.contains(tile),
        }
    }
}

enum Goals<'a> {
    Single {
        to: WorldTile,
        cost: Option<TargetCost>,
    },
    /// A preferred and a fallback goal set, each with its own budget and
    /// backward proof ([`FirstGoals`]); boxed, as its two proofs' inline
    /// state dwarfs the other shapes.
    First(Box<FirstGoals<'a>>),
    Many {
        unique: &'a HashMap<WorldTile, usize>,
        costs: &'a mut [Result<TargetCost, TargetError>],
        remaining: usize,
    },
}

/// What one settled node decided for the search.
enum Settle {
    Continue,
    /// Stop with the node counted: it decided the goals.
    Stop,
    /// Stop without counting the node: no goal set may spend it.
    Spent,
}

impl Goals<'_> {
    fn settle(&mut self, tile: WorldTile, ticks: f64, settled_at: usize) -> Settle {
        let cost = TargetCost { ticks, settled_at };
        let done = match self {
            Goals::Single { to, cost: found } => {
                if *to == tile {
                    *found = Some(cost);
                    true
                } else {
                    false
                }
            }
            Goals::First(first) => return first.settle(tile, cost),
            Goals::Many {
                unique,
                costs,
                remaining,
            } => {
                if let Some(&index) = unique.get(&tile) {
                    if costs[index].is_err() {
                        costs[index] = Ok(cost);
                        *remaining -= 1;
                    }
                }
                *remaining == 0
            }
        };
        if done {
            Settle::Stop
        } else {
            Settle::Continue
        }
    }

    /// Everything reachable has settled, `settled` nodes in all.
    fn exhausted(&mut self, settled: usize) {
        if let Goals::First(first) = self {
            first.exhausted(settled);
        }
    }

    /// The backward proofs' final report, dropping their scratch.
    fn finish_proofs(&mut self) -> ReverseReport {
        match self {
            Goals::First(first) => first.finish_proofs(),
            Goals::Single { .. } | Goals::Many { .. } => ReverseReport::default(),
        }
    }
}

/// One goal set of a first-goal search: its first goal to settle, the
/// settles it may spend, and its backward proof.
struct GoalSet<'a> {
    targets: FirstTargets<'a>,
    seeds: &'a [WorldTile],
    found: Option<(WorldTile, TargetCost)>,
    /// The unproven budget until the proof shows a goal reachable, then the
    /// reachable budget.
    budget: usize,
    proof: ReverseRun<'a>,
    /// Why no goal of the set can settle within its budget.
    closed: Option<RouteError>,
}

impl<'a> GoalSet<'a> {
    /// An empty set is closed from the start.
    fn new(targets: &'a [WorldTile], budget: usize) -> Self {
        Self {
            targets: FirstTargets::new(targets),
            seeds: targets,
            found: None,
            budget,
            proof: ReverseRun::default(),
            closed: targets.is_empty().then_some(RouteError::NoPath),
        }
    }

    fn live(&self) -> bool {
        self.found.is_none() && self.closed.is_none()
    }

    fn close(&mut self, error: RouteError) {
        self.closed = Some(error);
        self.proof.finish();
    }

    /// Records the set's first goal to settle, while it is still searching.
    fn accept(&mut self, tile: WorldTile, cost: TargetCost) -> bool {
        if !self.live() || !self.targets.contains(&tile) {
            return false;
        }
        self.found = Some((tile, cost));
        self.proof.finish();
        true
    }

    /// Runs the proof to one backward step per settle so far: reachability
    /// lifts the budget, unreachability closes the set.
    fn prove_to(&mut self, gates: ProofGates<'a>, steps: usize, reachable_budget: usize) {
        match self.proof.run_to(gates, self.seeds, steps) {
            Some(ReverseProof::Reachable) => self.budget = self.budget.max(reachable_budget),
            Some(ReverseProof::Unreachable) => self.close(RouteError::NoPath),
            _ => {}
        }
    }
}

/// A first-goal search's preferred and fallback goal sets. The preferred
/// set runs the search: its proof steps once per settle and lifts or ends
/// it, and its first goal to settle ends it. The fallback set records its
/// first goal on the way, decided at every point exactly as a search over
/// the fallback alone would decide it (Dijkstra's settle order does not
/// depend on the goals). Within its unproven budget that needs no proof;
/// when one of its goals settles past that budget, or the preferred set
/// stops without a goal, its proof starts, caught up to one step per settle
/// spent. The preferred proof has decided by then, so only one proof's
/// scratch is live at a time.
struct FirstGoals<'a> {
    gates: ProofGates<'a>,
    reachable_budget: usize,
    preferred: GoalSet<'a>,
    fallback: GoalSet<'a>,
}

impl<'a> FirstGoals<'a> {
    fn new(
        gates: ProofGates<'a>,
        targets: &'a [WorldTile],
        fallback: &'a [WorldTile],
        budget: usize,
        reachable_budget: usize,
    ) -> Self {
        let mut preferred = GoalSet::new(targets, budget);
        preferred.prove_to(gates, 0, reachable_budget);
        Self {
            gates,
            reachable_budget,
            preferred,
            fallback: GoalSet::new(fallback, budget),
        }
    }

    fn settle(&mut self, tile: WorldTile, cost: TargetCost) -> Settle {
        let settled = cost.settled_at;
        if settled > self.preferred.budget {
            self.preferred.close(RouteError::BudgetExhausted);
            self.decide_fallback(settled - 1, settled);
            return Settle::Spent;
        }
        if self.preferred.accept(tile, cost) {
            return Settle::Stop;
        }
        self.settle_fallback(tile, cost);
        self.preferred
            .prove_to(self.gates, settled, self.reachable_budget);
        if self.preferred.live() {
            return Settle::Continue;
        }
        self.decide_fallback(settled, settled);
        Settle::Stop
    }

    /// A fallback goal settling past the set's unproven budget counts only
    /// if the set's proof showed a goal reachable within that budget;
    /// otherwise a search over the fallback alone stopped at the budget, and
    /// so does the set. (Settling past the preferred set's lifted budget
    /// implies the preferred proof decided, dropping its scratch.)
    fn settle_fallback(&mut self, tile: WorldTile, cost: TargetCost) {
        if !self.fallback.live() || !self.fallback.targets.contains(&tile) {
            return;
        }
        let settled = cost.settled_at;
        if settled > self.fallback.budget {
            self.decide_fallback(settled - 1, settled);
        }
        self.fallback.accept(tile, cost);
    }

    /// Decides the fallback set, where that needs no further settle, after
    /// `spent` settles with settle ordinal `known` known to exist. Its proof
    /// is caught up to one step per settle spent, up to its budget, as a
    /// search over it alone would have stepped it: shown unreachable, the
    /// set has failed; not shown reachable while a settle past its budget
    /// exists, that search stopped at the budget. Otherwise it stays
    /// undecided.
    fn decide_fallback(&mut self, spent: usize, known: usize) {
        if !self.fallback.live() {
            return;
        }
        let steps = spent.min(self.fallback.budget);
        self.fallback
            .prove_to(self.gates, steps, self.reachable_budget);
        if self.fallback.live() && known > self.fallback.budget {
            self.fallback.close(RouteError::BudgetExhausted);
        }
    }

    /// Everything reachable settled, `settled` nodes, without a preferred
    /// goal; a fallback set not stopped earlier has no goal to settle.
    fn exhausted(&mut self, settled: usize) {
        self.preferred.close(RouteError::NoPath);
        self.decide_fallback(settled, settled);
        if self.fallback.live() {
            self.fallback.close(RouteError::NoPath);
        }
    }

    /// The preferred proof's outcome, and the larger scratch of the two
    /// proofs (the fallback's starts only after the preferred one's is
    /// dropped).
    fn finish_proofs(&mut self) -> ReverseReport {
        let preferred = self.preferred.proof.finish();
        let fallback = self.fallback.proof.finish();
        ReverseReport {
            proof: preferred.proof,
            seen: preferred.seen.max(fallback.seen),
            queue: preferred.queue.max(fallback.queue),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SearchStop {
    Completed,
    Exhausted,
    Budget,
    Deadline,
}

struct SearchOutcome {
    came_from: HashMap<WorldTile, Back>,
    settled: usize,
    stop: SearchStop,
    capacities: SearchCapacities,
    proof: ReverseProof,
}

impl SearchOutcome {
    fn empty() -> Self {
        Self {
            came_from: HashMap::new(),
            settled: 0,
            stop: SearchStop::Completed,
            capacities: SearchCapacities::default(),
            proof: ReverseProof::NotRun,
        }
    }

    #[allow(clippy::too_many_arguments)] // the kernel's scratch tables plus its stop facts
    fn finish(
        came_from: HashMap<WorldTile, Back>,
        dist: &HashMap<WorldTile, f64>,
        done: &HashSet<WorldTile>,
        heap: &BinaryHeap<HeapNode>,
        settled: usize,
        stop: SearchStop,
        record_capacities: bool,
        reverse: ReverseReport,
    ) -> Self {
        let capacities = if record_capacities {
            SearchCapacities {
                distances: dist.capacity(),
                predecessors: came_from.capacity(),
                settled: done.capacity(),
                heap: heap.capacity(),
                reverse: reverse.seen,
                reverse_queue: reverse.queue,
            }
        } else {
            SearchCapacities::default()
        };
        Self {
            came_from,
            settled,
            stop,
            capacities,
            proof: reverse.proof,
        }
    }
}

/// Whether `state` proves `edge`'s requirements; the BankBudget diagnosis
/// (`relax_carry_worn`) ignores only its `item_req`/`worn_req` gates.
fn edge_allowed(state: &WorldState, edge: &TransportEdge, relax_carry_worn: bool) -> bool {
    if relax_carry_worn {
        state.allows_without_carry_worn(edge)
    } else {
        state.allows(edge)
    }
}

/// The outcome and peak scratch of one search's backward proof.
#[derive(Clone, Copy, Default)]
struct ReverseReport {
    proof: ReverseProof,
    seen: usize,
    queue: usize,
}

/// The forward search's gates, as its backward proofs borrow them.
#[derive(Clone, Copy)]
struct ProofGates<'a> {
    collision: &'a WorldCollision,
    graph: &'a TransportGraph,
    state: &'a WorldState,
    essence: Option<&'a EssenceSession>,
    from: WorldTile,
    use_teleports: bool,
    allow_wilderness: bool,
    relax_carry_worn: bool,
}

/// One goal set's backward proof: not yet started, then live closure
/// scratch until the proof decides, then only its report.
#[derive(Default)]
struct ReverseRun<'a> {
    started: bool,
    /// Backward steps taken since the proof started.
    steps: usize,
    closure: Option<ReverseClosure<'a>>,
    report: ReverseReport,
}

impl<'a> ReverseRun<'a> {
    /// Starts the proof from `seeds` if it has not started, then steps it
    /// until it has taken `steps` backward steps. A decision drops the
    /// closure's scratch and is returned once.
    fn run_to(
        &mut self,
        gates: ProofGates<'a>,
        seeds: &[WorldTile],
        steps: usize,
    ) -> Option<ReverseProof> {
        if !self.started {
            self.started = true;
            let mut closure = ReverseClosure::new(gates);
            for &seed in seeds {
                if let Some(proof) = closure.admit(seed) {
                    self.report = closure.report(proof);
                    return Some(proof);
                }
            }
            self.closure = Some(closure);
        }
        while self.steps < steps {
            let closure = self.closure.as_mut()?;
            self.steps += 1;
            if let Some(proof) = closure.step() {
                self.report = closure.report(proof);
                self.closure = None;
                return Some(proof);
            }
        }
        None
    }

    /// The final report, dropping live scratch: a closure still running
    /// when its set or the search stops is [`ReverseProof::Open`].
    fn finish(&mut self) -> ReverseReport {
        if let Some(closure) = self.closure.take() {
            self.report = closure.report(ReverseProof::Open);
        }
        self.report
    }
}

/// Backward closure of a goal set over a superset of the forward search's
/// moves. Walk predecessors are exact: `p` precedes `q` when the forward
/// [`step_ok`] and wilderness entry gate admit the step `p -> q`. Transport
/// and essence-return predecessors (the standable take-offs within
/// [`INTERACT_RADIUS`] of each usable edge landing in the closure) are added
/// whenever the walk frontier empties, by one scan of the packed edge list (a
/// few thousand edges) rather than a per-search index. Avoidance rectangles
/// only remove forward moves and are ignored.
///
/// Admitting the origin, or the landing of a teleport the forward search can
/// take from the origin, proves a goal reachable. Any other usable teleport
/// landing in the closure makes every tile a potential predecessor, so a
/// closure holding one cannot prove anything unreachable. Otherwise a closure
/// that empties without meeting the origin is closed under every forward
/// predecessor, so no forward path reaches the goal set.
struct ReverseClosure<'a> {
    collision: &'a WorldCollision,
    graph: &'a TransportGraph,
    state: &'a WorldState,
    essence: Option<&'a EssenceSession>,
    from: WorldTile,
    use_teleports: bool,
    allow_wilderness: bool,
    relax_carry_worn: bool,
    /// Landings of the gated teleports usable from `from` itself.
    landings: HashSet<WorldTile>,
    seen: HashSet<WorldTile>,
    queue: VecDeque<WorldTile>,
}

impl<'a> ReverseClosure<'a> {
    fn new(gates: ProofGates<'a>) -> Self {
        let ProofGates {
            collision,
            graph,
            state,
            essence,
            from,
            use_teleports,
            allow_wilderness,
            relax_carry_worn,
        } = gates;
        let landings = if use_teleports {
            let level = graph.wilderness.level(from);
            graph
                .teleports
                .iter()
                .filter(|edge| {
                    edge_allowed(state, edge, relax_carry_worn)
                        && TransportGraph::teleport_legal_at_level(level, edge)
                        && wildy_step_ok(graph, from, edge.to, allow_wilderness)
                })
                .map(|edge| edge.to)
                .collect()
        } else {
            HashSet::new()
        };
        Self {
            collision,
            graph,
            state,
            essence,
            from,
            use_teleports,
            allow_wilderness,
            relax_carry_worn,
            landings,
            seen: HashSet::new(),
            queue: VecDeque::new(),
        }
    }

    fn report(&self, proof: ReverseProof) -> ReverseReport {
        ReverseReport {
            proof,
            seen: self.seen.capacity(),
            queue: self.queue.capacity(),
        }
    }

    /// Admit a predecessor. `Some` ends the proof: the origin (or a teleport
    /// landing it can reach directly) precedes the goals, or the closure
    /// outgrew [`REVERSE_PROOF_BUDGET`].
    fn admit(&mut self, tile: WorldTile) -> Option<ReverseProof> {
        if !self.seen.insert(tile) {
            return None;
        }
        if tile == self.from || (!self.landings.is_empty() && self.landings.contains(&tile)) {
            return Some(ReverseProof::Reachable);
        }
        if self.seen.len() > REVERSE_PROOF_BUDGET {
            return Some(ReverseProof::Abandoned);
        }
        self.queue.push_back(tile);
        None
    }

    /// Expand the oldest frontier tile's walk predecessors, or cross
    /// transports once the walk frontier is empty.
    fn step(&mut self) -> Option<ReverseProof> {
        let Some(tile) = self.queue.pop_front() else {
            return self.cross_transports();
        };
        for d in STEPS {
            let before = WorldTile {
                x: tile.x - d.0,
                z: tile.z - d.1,
                level: tile.level,
            };
            if step_ok(self.collision, before, d)
                && wildy_step_ok(self.graph, before, tile, self.allow_wilderness)
            {
                if let Some(proof) = self.admit(before) {
                    return Some(proof);
                }
            }
        }
        None
    }

    /// Admit every transport and essence-return take-off whose landing is in
    /// the closure. Nothing new means the closure is complete.
    fn cross_transports(&mut self) -> Option<ReverseProof> {
        let graph = self.graph;
        let state = self.state;
        if self.use_teleports
            && graph.teleports.iter().any(|edge| {
                self.seen.contains(&edge.to) && edge_allowed(state, edge, self.relax_carry_worn)
            })
        {
            return Some(ReverseProof::Abandoned);
        }
        let admitted = self.seen.len();
        for edge in &graph.edges {
            if self.seen.contains(&edge.to) && edge_allowed(state, edge, self.relax_carry_worn) {
                if let Some(proof) = self.admit_takeoffs(edge.at, edge.to) {
                    return Some(proof);
                }
            }
        }
        if let Some(session) = self.essence {
            if self.seen.contains(&session.return_tile) {
                for &portal in ESSENCE_MINE_PORTALS {
                    if let Some(proof) = self.admit_takeoffs(portal, session.return_tile) {
                        return Some(proof);
                    }
                }
            }
        }
        (self.seen.len() == admitted).then_some(ReverseProof::Unreachable)
    }

    /// The forward search takes an edge at `at` from any standable tile
    /// within [`INTERACT_RADIUS`] on `at`'s level.
    fn admit_takeoffs(&mut self, at: WorldTile, to: WorldTile) -> Option<ReverseProof> {
        for dx in -INTERACT_RADIUS..=INTERACT_RADIUS {
            for dz in -INTERACT_RADIUS..=INTERACT_RADIUS {
                let takeoff = WorldTile {
                    x: at.x + dx,
                    z: at.z + dz,
                    level: at.level,
                };
                if self.collision.standable(takeoff)
                    && wildy_step_ok(self.graph, takeoff, to, self.allow_wilderness)
                {
                    if let Some(proof) = self.admit(takeoff) {
                        return Some(proof);
                    }
                }
            }
        }
        None
    }
}

/// The shared single-target Dijkstra behind [`find`]/[`find_with`]:
/// `use_teleports` unions the any-tile teleport layer into the relaxation
/// from every settled node. Transport edges are relaxed from any standable
/// tile within [`INTERACT_RADIUS`] of their `at` (never from `at` itself
/// when it is blocked); walk steps are the strict directional [`step_ok`]
/// test throughout. `allow_wilderness` gates stepping into (or landing in)
/// the wilderness zone; `state` gates every transport edge (walked or
/// teleported) on its requirements — an edge the state cannot prove is not
/// relaxed.
#[allow(clippy::too_many_arguments)]
fn find_bounded_impl(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
    model: CostModel,
    budget: usize,
    use_teleports: bool,
    allow_wilderness: bool,
    state: &WorldState,
    essence: Option<&EssenceSession>,
    avoid: &[AvoidRect],
) -> Result<Route, RouteError> {
    if from == to {
        return Ok(Route {
            legs: vec![Leg::Walk { tiles: vec![from] }],
            dest: to,
            ticks: 0.0,
        });
    }

    let mut goals = Goals::Single { to, cost: None };
    let search = search_kernel(
        collision,
        graph,
        from,
        model,
        budget,
        use_teleports,
        allow_wilderness,
        state,
        essence,
        false,
        avoid,
        &mut goals,
        None,
    );
    match goals {
        Goals::Single {
            cost: Some(cost), ..
        } => {
            let (legs, ticks) = reconstruct(to, &search.came_from, graph, model, essence);
            debug_assert_eq!(ticks, cost.ticks);
            Ok(Route {
                legs,
                dest: to,
                ticks,
            })
        }
        _ if search.stop == SearchStop::Budget => Err(RouteError::BudgetExhausted),
        _ => Err(RouteError::NoPath),
    }
}

/// The Dijkstra kernel shared by every search shape. `relax_carry_worn` is
/// the BankBudget diagnosis arm only: it drops the `item_req`/`worn_req`
/// gates so the session can tell a missing-item failure from a
/// skill/quest/varp gate. `budget` caps every search; within it, `goals`
/// decide at each settled node whether the search goes on (a first-goal
/// search's sets run their own budgets and backward proofs).
#[allow(clippy::too_many_arguments)]
fn search_kernel(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    model: CostModel,
    budget: usize,
    use_teleports: bool,
    allow_wilderness: bool,
    state: &WorldState,
    essence: Option<&EssenceSession>,
    relax_carry_worn: bool,
    avoid: &[AvoidRect],
    goals: &mut Goals<'_>,
    deadline: Option<Instant>,
) -> SearchOutcome {
    let record_capacities = matches!(goals, Goals::First(_) | Goals::Many { .. });
    let mut dist: HashMap<WorldTile, f64> = HashMap::new();
    let mut came_from: HashMap<WorldTile, Back> = HashMap::new();
    let mut heap: BinaryHeap<HeapNode> = BinaryHeap::new();
    let mut done: HashSet<WorldTile> = HashSet::new();

    dist.insert(from, 0.0);
    heap.push(HeapNode {
        cost: 0.0,
        tile: from,
    });

    let mut expanded = 0usize;
    let mut heap_pops = 0usize;
    let expired = || deadline.is_some_and(|end| Instant::now() >= end);
    while !heap.is_empty() {
        if heap_pops & 255 == 0 && expired() {
            return SearchOutcome::finish(
                came_from,
                &dist,
                &done,
                &heap,
                expanded,
                SearchStop::Deadline,
                record_capacities,
                goals.finish_proofs(),
            );
        }
        heap_pops += 1;
        let n = heap.pop().expect("nonempty heap");
        let cur = n.tile;
        // A stale heap entry (a cheaper path was found after the push) is
        // skipped; the first pop at the settled distance settles the tile.
        if dist.get(&cur) != Some(&n.cost) {
            continue;
        }
        if !done.insert(cur) {
            continue;
        }
        expanded += 1;
        let stop = if expanded > budget {
            Some((budget, SearchStop::Budget))
        } else {
            match goals.settle(cur, n.cost, expanded) {
                Settle::Continue => None,
                Settle::Stop => Some((expanded, SearchStop::Completed)),
                Settle::Spent => Some((expanded - 1, SearchStop::Budget)),
            }
        };
        if let Some((settled, stop)) = stop {
            return SearchOutcome::finish(
                came_from,
                &dist,
                &done,
                &heap,
                settled,
                if expired() {
                    SearchStop::Deadline
                } else {
                    stop
                },
                record_capacities,
                goals.finish_proofs(),
            );
        }

        let escaping = !avoid.is_empty() && tile_in_any_avoid(cur, avoid);

        for d in STEPS {
            if step_ok(collision, cur, d) {
                let nb = WorldTile {
                    x: cur.x + d.0,
                    z: cur.z + d.1,
                    level: cur.level,
                };
                if !avoid.is_empty() && !escaping && tile_in_any_avoid(nb, avoid) {
                    continue;
                }
                if !wildy_step_ok(graph, cur, nb, allow_wilderness) {
                    continue;
                }
                let nd = n.cost + model.run_per_step;
                if !done.contains(&nb) && dist.get(&nb).is_none_or(|&g| g > nd) {
                    dist.insert(nb, nd);
                    came_from.insert(nb, Back::Walk(cur));
                    heap.push(HeapNode { cost: nd, tile: nb });
                }
            }
        }
        // Transport edges are usable from any standable tile within the
        // interact radius of their `at` — the approach is derived here,
        // never baked. `at` itself is the interact target and may be
        // blocked (a wall loc or NPC); only the take-off tile needs to be
        // standable. Each edge is indexed under its unique `at`, so the
        // fixed offset sweep finds it exactly once per node (a radius-1
        // square may cover several `at` tiles — each is a distinct edge).
        if collision.standable(cur) {
            for dx in -INTERACT_RADIUS..=INTERACT_RADIUS {
                for dz in -INTERACT_RADIUS..=INTERACT_RADIUS {
                    let at = WorldTile {
                        x: cur.x + dx,
                        z: cur.z + dz,
                        level: cur.level,
                    };
                    let Some(idxs) = graph.at.get(&at) else {
                        continue;
                    };
                    for &ei in idxs {
                        let edge = &graph.edges[ei];
                        if !edge_allowed(state, edge, relax_carry_worn) {
                            continue;
                        }
                        if !avoid.is_empty() && !escaping && tile_in_any_avoid(edge.to, avoid) {
                            continue;
                        }
                        if !wildy_step_ok(graph, cur, edge.to, allow_wilderness) {
                            continue;
                        }
                        let nd = n.cost + edge.ticks as f64;
                        if !done.contains(&edge.to) && dist.get(&edge.to).is_none_or(|&g| g > nd) {
                            dist.insert(edge.to, nd);
                            came_from.insert(edge.to, Back::Transport { from: cur, ei });
                            heap.push(HeapNode {
                                cost: nd,
                                tile: edge.to,
                            });
                        }
                    }
                }
            }
            // The essence-mine return: never packed — the pack carries
            // wizard → mine-pad entry edges only. When a session is
            // latched, each mine exit portal placement is relaxed from any
            // standable tile within the interact radius, landing on the
            // entry wizard's overworld anchor. Without a session the mine
            // is a sealed dead end, so `find` never treats it as a
            // corridor between arbitrary overworld tiles.
            if let Some(session) = essence {
                for (portal, &at) in ESSENCE_MINE_PORTALS.iter().enumerate() {
                    if cur.level != at.level
                        || (cur.x - at.x).abs().max((cur.z - at.z).abs()) > INTERACT_RADIUS
                    {
                        continue;
                    }
                    if !avoid.is_empty()
                        && !escaping
                        && tile_in_any_avoid(session.return_tile, avoid)
                    {
                        continue;
                    }
                    if !wildy_step_ok(graph, cur, session.return_tile, allow_wilderness) {
                        continue;
                    }
                    let nd = n.cost + ESSENCE_MINE_EXIT_TICKS as f64;
                    if !done.contains(&session.return_tile)
                        && dist.get(&session.return_tile).is_none_or(|&g| g > nd)
                    {
                        dist.insert(session.return_tile, nd);
                        came_from.insert(
                            session.return_tile,
                            Back::EssenceReturn { from: cur, portal },
                        );
                        heap.push(HeapNode {
                            cost: nd,
                            tile: session.return_tile,
                        });
                    }
                }
            }
        }
        // The any-tile teleport layer: every teleport edge leaves from the
        // current node, wherever it is. The landing is trusted like every
        // other transport `to` (no walkability filter — the content
        // declares it).
        if use_teleports {
            let wildy_level = graph.wilderness.level(cur);
            for (ti, edge) in graph.teleports.iter().enumerate() {
                if !edge_allowed(state, edge, relax_carry_worn) {
                    continue;
                }
                if !avoid.is_empty() && !escaping && tile_in_any_avoid(edge.to, avoid) {
                    continue;
                }
                if !wildy_step_ok(graph, cur, edge.to, allow_wilderness) {
                    continue;
                }
                if !TransportGraph::teleport_legal_at_level(wildy_level, edge) {
                    continue;
                }
                let nd = n.cost + edge.ticks as f64;
                if !done.contains(&edge.to) && dist.get(&edge.to).is_none_or(|&g| g > nd) {
                    dist.insert(edge.to, nd);
                    came_from.insert(
                        edge.to,
                        Back::Teleport {
                            from: cur,
                            index: ti,
                        },
                    );
                    heap.push(HeapNode {
                        cost: nd,
                        tile: edge.to,
                    });
                }
            }
        }
    }
    let stop = if expired() {
        SearchStop::Deadline
    } else {
        goals.exhausted(expanded);
        SearchStop::Exhausted
    };
    SearchOutcome::finish(
        came_from,
        &dist,
        &done,
        &heap,
        expanded,
        stop,
        record_capacities,
        goals.finish_proofs(),
    )
}

/// Whether the search may move from `cur` onto `next`: without
/// `allow_wilderness` a non-wilderness node may not relax into a
/// wilderness tile (a walk step or a transport landing). Once inside the
/// wilderness the search walks freely — only the entry is gated. Membership
/// is the packed [`TransportGraph::wilderness`] table; empty zones gate
/// nothing.
fn wildy_step_ok(
    graph: &TransportGraph,
    cur: WorldTile,
    next: WorldTile,
    allow_wilderness: bool,
) -> bool {
    allow_wilderness || graph.wilderness.contains(cur) || !graph.wilderness.contains(next)
}

/// Whether a one-tile step from `cur` by `d` is allowed — the client's
/// `tryMove` movement test against the collision bake's derived walkable
/// word, not the blanket `walkable()`. A step into a neighbour clears that
/// neighbour's `PL_WALK_*` mask for the face/corner the step enters
/// through; a diagonal step additionally clears both orthogonal neighbours'
/// cardinal masks, exactly like `tryMove`'s BFS. The derived word carries
/// the `SQ_BLOCKED` base on any wall/scenery/ground tile, so those tiles
/// reject entry from every direction. Every step stays inside the bake's
/// x/z grid (the whole-world mapsquare bbox); each level tests its own
/// plane (an unstamped plane is unrestricted within it).
pub(crate) fn step_ok(collision: &WorldCollision, cur: WorldTile, d: (i32, i32)) -> bool {
    let nb = WorldTile {
        x: cur.x + d.0,
        z: cur.z + d.1,
        level: cur.level,
    };
    let lx = nb.x - collision.origin.x;
    let lz = nb.z - collision.origin.z;
    if lx < 0 || lz < 0 {
        return false;
    }
    if lx as usize >= collision.width || lz as usize >= collision.height {
        return false;
    }
    let f = |x: i32, z: i32| collision.walkable_word(x, z, nb.level);
    if let Some(mask) = cardinal_entry_mask(d) {
        // Cardinal: the destination's face toward `cur`.
        return f(nb.x, nb.z) & mask == 0;
    }
    match (d.0, d.1) {
        // Diagonal: the destination's corner mask plus both orthogonals.
        (-1, -1) => {
            f(nb.x, nb.z) & MASK_NE == 0
                && f(cur.x - 1, cur.z) & MASK_E == 0
                && f(cur.x, cur.z - 1) & MASK_N == 0
        }
        (1, -1) => {
            f(nb.x, nb.z) & MASK_NW == 0
                && f(cur.x + 1, cur.z) & MASK_W == 0
                && f(cur.x, cur.z - 1) & MASK_N == 0
        }
        (-1, 1) => {
            f(nb.x, nb.z) & MASK_SE == 0
                && f(cur.x - 1, cur.z) & MASK_E == 0
                && f(cur.x, cur.z + 1) & MASK_S == 0
        }
        (1, 1) => {
            f(nb.x, nb.z) & MASK_SW == 0
                && f(cur.x + 1, cur.z) & MASK_W == 0
                && f(cur.x, cur.z + 1) & MASK_S == 0
        }
        _ => false,
    }
}

/// The `PL_WALK_*` mask a cardinal step by `d` tests on the tile it enters
/// (the face toward the tile it leaves); `None` for any other delta.
pub(crate) fn cardinal_entry_mask(d: (i32, i32)) -> Option<u32> {
    match d {
        (0, 1) => Some(MASK_S),
        (0, -1) => Some(MASK_N),
        (1, 0) => Some(MASK_W),
        (-1, 0) => Some(MASK_E),
        _ => None,
    }
}

/// Return the component reachable from `origin` using baked client-style
/// walk steps within the requested Chebyshev radius. This deliberately omits
/// transports and validates its own origin so callers cannot turn it into an
/// unbounded or cross-plane flood.
pub fn local_step_component(
    collision: &WorldCollision,
    origin: WorldTile,
    radius: i32,
) -> HashSet<WorldTile> {
    let mut visited = HashSet::new();
    let r = radius.clamp(0, 104);
    if !collision.standable(origin) {
        return visited;
    }
    let mut queue = std::collections::VecDeque::from([origin]);
    visited.insert(origin);
    while let Some(cur) = queue.pop_front() {
        for d in STEPS {
            let next = WorldTile {
                x: cur.x + d.0,
                z: cur.z + d.1,
                level: origin.level,
            };
            if (next.x - origin.x).abs().max((next.z - origin.z).abs()) > r
                || next.level != origin.level
                || !collision.standable(next)
                || !step_ok(collision, cur, d)
                || !visited.insert(next)
            {
                continue;
            }
            queue.push_back(next);
        }
    }
    visited
}

/// How a tile was reached: by a walk step (each costing the search's run
/// rate) from `Walk`'s tile, by transport edge `Transport` (an index
/// into [`TransportGraph::edges`], taken from `from` — the standable tile
/// within the edge's interact radius that it was relaxed from), by
/// any-tile teleport edge `Teleport` (an index into
/// [`TransportGraph::teleports`], taken from `from` — the node it was
/// relaxed from), or by the session-gated essence-mine return hop
/// `EssenceReturn` (the mine exit portal placement `portal` was taken
/// from `from`).
#[derive(Clone, Copy)]
enum Back {
    Walk(WorldTile),
    Transport { from: WorldTile, ei: usize },
    Teleport { from: WorldTile, index: usize },
    EssenceReturn { from: WorldTile, portal: usize },
}

/// Split the backtrack from `to` back to the entry-less origin into legs:
/// consecutive walk tiles collapse into one `Walk` leg per run, and each
/// transport edge is its own `Transport` leg. Returns `(legs, ticks)`
/// where `ticks` is the total: `(walk tiles − 1) × run rate` per walk leg
/// plus each transport edge's ticks.
fn reconstruct(
    to: WorldTile,
    came_from: &HashMap<WorldTile, Back>,
    graph: &TransportGraph,
    model: CostModel,
    essence: Option<&EssenceSession>,
) -> (Vec<Leg>, f64) {
    // Walk tiles in backtrack order (dest side first).
    let mut walk_rev = vec![to];
    let mut t = to;
    let mut legs_rev: Vec<Leg> = Vec::new();
    let mut ticks = 0.0;
    while let Some(prev) = came_from.get(&t) {
        match *prev {
            Back::Walk(pt) => {
                walk_rev.push(pt);
                t = pt;
            }
            Back::Transport { from, ei } => {
                ticks += walk_ticks(&walk_rev, model);
                walk_rev.reverse();
                legs_rev.push(Leg::Walk { tiles: walk_rev });
                let edge = graph.edges[ei].clone();
                ticks += edge.ticks as f64;
                legs_rev.push(Leg::Transport { edge });
                // The walk leg before the transport resumes from the tile
                // the edge was actually taken on — the standable take-off
                // within the interact radius, never the edge's `at` (which
                // may be a blocked interact target the player cannot stand
                // on).
                t = from;
                walk_rev = vec![t];
            }
            Back::Teleport { from, index } => {
                ticks += walk_ticks(&walk_rev, model);
                walk_rev.reverse();
                legs_rev.push(Leg::Walk { tiles: walk_rev });
                let edge = graph.teleports[index].clone();
                ticks += edge.ticks as f64;
                legs_rev.push(Leg::Transport { edge });
                // The walk leg before the teleport resumes from the tile the
                // teleport was actually taken on (a teleport has no `at`).
                t = from;
                walk_rev = vec![t];
            }
            Back::EssenceReturn { from, portal } => {
                ticks += walk_ticks(&walk_rev, model);
                walk_rev.reverse();
                legs_rev.push(Leg::Walk { tiles: walk_rev });
                let session = essence.expect("essence return implies a latched session");
                let edge =
                    crate::essence::essence_return_edge(ESSENCE_MINE_PORTALS[portal], session);
                ticks += edge.ticks as f64;
                legs_rev.push(Leg::Transport { edge });
                // The walk leg before the return resumes from the take-off
                // tile the portal was relaxed from.
                t = from;
                walk_rev = vec![t];
            }
        }
    }
    ticks += walk_ticks(&walk_rev, model);
    walk_rev.reverse();
    legs_rev.push(Leg::Walk { tiles: walk_rev });
    legs_rev.reverse();
    (legs_rev, ticks)
}

/// The tick cost of one collapsed walk leg: `tiles.len() − 1` steps at the
/// model's run rate (the origin tile is stood on, not walked to).
fn walk_ticks(tiles: &[WorldTile], model: CostModel) -> f64 {
    tiles.len().saturating_sub(1) as f64 * model.run_per_step
}

/// Heap entry for relaxations over total tick cost; `Ord` is reversed so
/// the smallest cost pops first, with `total_cmp` giving f64 a total order
/// and tile coordinates as tie-breakers to keep the ordering total.
struct HeapNode {
    cost: f64,
    tile: WorldTile,
}

impl PartialEq for HeapNode {
    fn eq(&self, other: &Self) -> bool {
        self.cost == other.cost && self.tile == other.tile
    }
}
impl Eq for HeapNode {}

impl PartialOrd for HeapNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HeapNode {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .cost
            .total_cmp(&self.cost)
            .then_with(|| self.tile.x.cmp(&other.tile.x))
            .then_with(|| self.tile.z.cmp(&other.tile.z))
            .then_with(|| self.tile.level.cmp(&other.tile.level))
    }
}

mod grid;
pub use grid::{find_on_grid, GridLeg, GridRoute, NoPath};

#[cfg(test)]
#[path = "router_tests.rs"]
mod tests;
