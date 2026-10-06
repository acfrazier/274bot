use std::borrow::Cow;
use std::collections::VecDeque;

use api::bank_memory::Origin;
use api::snapshot::WorldTile;
use nav::bank_fetch::{fetchable_state, plan_bank_fetch, planning_rows, BankFetch, BankStep};
use nav::router::{
    find_first_missing_item_reqs_with_avoid, find_first_with_avoid, find_first_with_fallback_avoid,
    find_missing_item_reqs_with_avoid, find_with_avoid, missing_item_reqs, AvoidRect,
    FallbackRoute, FindOptions, Route,
};
use nav::transport::TransportEdge;
use nav::world::NavWorld;
use nav::WorldState;

/// A latched BankBudget session: remaining [`BankStep`]s plus the dest
/// the arm re-finds after the session lands. Follow freezes only for
/// Open / Withdraw / Withdraw-X / Wear / Close; [`BankStep::Walk`] follows
/// the access-tile sub-route. Wear-from-inventory and bank-trip withdrawals
/// pump through the same path. `final_route` is the post-session route
/// (status row + follow once steps clear); a Walk-to-access may temporarily
/// replace `WalkArm::route` / `NavBot::route`. The front step's
/// [`StepProgress`] lives here too, so the script pump (`NavBot`) and the
/// panel/TUI pump (`WalkArm`) share one budget and one in-flight latch
/// across pumps.
#[derive(Debug, Clone)]
pub struct PendingBankFetch {
    pub steps: VecDeque<BankStep>,
    pub dest: WorldTile,
    pub opts: FindOptions,
    pub final_route: Route,
    /// The walk's avoidance rectangles: the access sub-route keeps out of
    /// them as the walk does.
    pub avoid: Vec<AvoidRect>,
    /// The front step's pump budget and in-flight send.
    pub progress: StepProgress,
}

impl PendingBankFetch {
    /// Pop the front step: the next one starts with a fresh budget and no
    /// send in flight.
    pub(crate) fn pop_step(&mut self) {
        self.steps.pop_front();
        self.progress = StepProgress::default();
    }
}

/// The front non-Walk step's progress: pumps spent waiting for it to land,
/// and the snapshot facts its last send latched (boxed: most pumps have no
/// send in flight, and every planned session carries it).
#[derive(Debug, Clone, Default)]
pub struct StepProgress {
    pub(crate) attempts: u32,
    pub(crate) flight: Option<Box<BankFetchFlight>>,
}

/// The snapshot facts a BankBudget send latched, so the next pump does
/// not re-send until the bank session or the step's own target changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BankFetchFlight {
    pub(crate) bank_gen: u64,
    pub(crate) bank_com: i32,
    pub(crate) target: FlightTarget,
}

/// What a sent step moves, as the snapshot showed it at the send. A change
/// anywhere else (another backpack row, another worn obj) is not this
/// send's acknowledgement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FlightTarget {
    /// Open / Close: the bank session alone.
    Bank,
    /// Withdraw / Withdraw-X / Wear: the obj's backpack total and whether it is worn.
    Obj { id: i32, carried: i32, worn: bool },
}

/// Outcome of a walk-arm route attempt: a direct route, a BankBudget
/// session plus the post-session route, or no path.
pub(super) enum RouteOutcome {
    Routed(Route),
    BankSession {
        pending: PendingBankFetch,
        route: Route,
    },
    NoPath,
}

/// Strict `find_with`, then — only when `allow_bank_fetch` is on and the
/// failure is solely missing item/worn reqs — plan a BankBudget session
/// over [`planning_rows`] of the bank memory's `origin` and `bank` rows and
/// re-find against the session's post state. Never inserts a virtual bank
/// edge into Dijkstra. Every search keeps out of `avoid`.
#[allow(clippy::too_many_arguments)] // search surface plus the memory's origin and rows
pub(super) fn route_or_bank_fetch(
    world: &NavWorld,
    from: WorldTile,
    to: WorldTile,
    opts: FindOptions,
    state: &WorldState,
    origin: Origin,
    bank: &[(i32, i32)],
    avoid: &[AvoidRect],
) -> RouteOutcome {
    match find_with_avoid(&world.collision, &world.graph, from, to, opts, state, avoid) {
        Ok(route) => RouteOutcome::Routed(route),
        Err(_) if opts.allow_bank_fetch => find_missing_item_reqs_with_avoid(
            &world.collision,
            &world.graph,
            from,
            to,
            opts,
            state,
            avoid,
        )
        .and_then(|missing| {
            plan_bank_fetch(
                &missing,
                state,
                &planning_rows(origin, bank, &missing),
                world.banks(),
                from,
                &world.collision,
            )
        })
        .and_then(|fetch| session_route(world, from, to, fetch, opts, avoid))
        .unwrap_or(RouteOutcome::NoPath),
        Err(_) => RouteOutcome::NoPath,
    }
}

/// The facts a BankBudget session can establish ([`fetchable_state`]) for
/// a first-goal search over `stands` with the `tiles` fallback, when
/// BankBudget is on and they open a gate the real state refuses. A search
/// under facts that open nothing would only repeat the strict one.
/// `Session` and `Unknown` rows are the supply as they are. A `Hint` is
/// advisory (design-bank-snapshot §2.4): before its stock can reject a
/// goal, one goal-set diagnosis ([`find_first_missing_item_reqs_with_avoid`],
/// the strict first-goal search's budgets) names what the chosen goal's
/// route misses, and that diagnosis is overlaid on the hint
/// ([`planning_rows`]), so a goal whose only route crosses a gate the hint
/// lacks still reaches [`session_for`] and its one verifying trip.
#[allow(clippy::too_many_arguments)] // goal set, search surface and the memory's origin and rows
pub(super) fn fetchable_facts(
    world: &NavWorld,
    from: WorldTile,
    stands: &[WorldTile],
    tiles: &[WorldTile],
    opts: FindOptions,
    state: &WorldState,
    origin: Origin,
    bank: &[(i32, i32)],
    avoid: &[AvoidRect],
) -> Option<WorldState> {
    if !opts.allow_bank_fetch {
        return None;
    }
    let diagnosis = (origin == Origin::Hint && !world.banks().is_empty())
        .then(|| {
            find_first_missing_item_reqs_with_avoid(
                &world.collision,
                &world.graph,
                from,
                stands,
                tiles,
                opts,
                state,
                avoid,
            )
        })
        .flatten();
    let rows = diagnosis.as_deref().map_or(Cow::Borrowed(bank), |missing| {
        planning_rows(origin, bank, missing)
    });
    let fetchable = fetchable_state(state, &rows, world.banks());
    let opens_gate = |edge: &TransportEdge| fetchable.allows(edge) && !state.allows(edge);
    (world.graph.edges.iter().any(opens_gate)
        || opts.allow_teleports && world.graph.teleports.iter().any(opens_gate))
    .then_some(fetchable)
}

/// What the search for a stand a session can reach established.
pub(super) enum StandFetch {
    /// A session to a stand, or a strict route to one past the strict
    /// search's budget.
    Outcome(RouteOutcome),
    /// No stand: the same search's answer for the radius tiles, if any.
    Tiles(Option<FallbackRoute>),
}

/// After the strict search found no stand: one search under `fetchable`
/// finds the cheapest stand whose item and worn gates the bank and backpack
/// can meet, so a stand behind an obj the session cannot get never hides
/// one it can, recording `tiles` on the way as the strict search did. When a
/// session's post-state re-find refuses its stand (no session for its
/// missing facts, or the post state still misses a gate), only that stand
/// is dropped and the rest searched again, so every round shrinks the
/// stands. A session never deposits (design-bank-snapshot D7).
#[allow(clippy::too_many_arguments)] // search surface plus the session's facts
pub(super) fn fetch_stand(
    world: &NavWorld,
    from: WorldTile,
    stands: &[WorldTile],
    tiles: &[WorldTile],
    opts: FindOptions,
    state: &WorldState,
    fetchable: &WorldState,
    origin: Origin,
    bank: &[(i32, i32)],
    avoid: &[AvoidRect],
) -> StandFetch {
    let mut stands = stands.to_vec();
    loop {
        let search = find_first_with_fallback_avoid(
            &world.collision,
            &world.graph,
            from,
            &stands,
            tiles,
            opts,
            fetchable,
            avoid,
        );
        let route = match search.into_routes() {
            (Ok(route), _) => route,
            (Err(_), tile) => return StandFetch::Tiles(tile),
        };
        let target = route.dest;
        if let Some(outcome) = session_for(world, from, route, opts, state, origin, bank, avoid) {
            return StandFetch::Outcome(outcome);
        }
        stands.retain(|&tile| tile != target);
        if stands.is_empty() {
            return StandFetch::Tiles((!tiles.is_empty()).then_some(FallbackRoute::Undecided));
        }
    }
}

/// After no stand and no strict radius tile routed: the cheapest tile a
/// session can reach, from what [`fetch_stand`] recorded (`known`) or a
/// search over the tiles alone when that is undecided. A refused session
/// drops only its tile, as in [`fetch_stand`].
#[allow(clippy::too_many_arguments)] // search surface plus the session's facts
pub(super) fn fetch_tile(
    world: &NavWorld,
    from: WorldTile,
    tiles: &[WorldTile],
    known: Option<FallbackRoute>,
    opts: FindOptions,
    state: &WorldState,
    fetchable: &WorldState,
    origin: Origin,
    bank: &[(i32, i32)],
    avoid: &[AvoidRect],
) -> RouteOutcome {
    let mut tiles = tiles.to_vec();
    let mut known = known;
    loop {
        let route = match known.take() {
            Some(FallbackRoute::Routed(route)) => route,
            Some(FallbackRoute::Undecided) => match find_first_with_avoid(
                &world.collision,
                &world.graph,
                from,
                &tiles,
                opts,
                fetchable,
                avoid,
            )
            .into_route()
            {
                Ok(route) => route,
                Err(_) => return RouteOutcome::NoPath,
            },
            Some(FallbackRoute::Failed(_)) | None => return RouteOutcome::NoPath,
        };
        let target = route.dest;
        if let Some(outcome) = session_for(world, from, route, opts, state, origin, bank, avoid) {
            return outcome;
        }
        tiles.retain(|&tile| tile != target);
        if tiles.is_empty() {
            return RouteOutcome::NoPath;
        }
        known = Some(FallbackRoute::Undecided);
    }
}

/// A route found under the fetchable facts: taken as found when it needs no
/// missing fact (the strict search only ran out of budget), else the
/// planned session to its goal, if the post-state re-find allows it.
#[allow(clippy::too_many_arguments)] // search surface plus the session's facts
fn session_for(
    world: &NavWorld,
    from: WorldTile,
    route: Route,
    opts: FindOptions,
    state: &WorldState,
    origin: Origin,
    bank: &[(i32, i32)],
    avoid: &[AvoidRect],
) -> Option<RouteOutcome> {
    let missing = missing_item_reqs(&route, state);
    if missing.is_empty() {
        return Some(RouteOutcome::Routed(route));
    }
    let fetch = plan_bank_fetch(
        &missing,
        state,
        &planning_rows(origin, bank, &missing),
        world.banks(),
        from,
        &world.collision,
    )?;
    session_route(world, from, route.dest, fetch, opts, avoid)
}

/// Re-find `to` against a planned session's post state (ADR 0005: find
/// itself stays fail-closed; the session is what unblocks).
fn session_route(
    world: &NavWorld,
    from: WorldTile,
    to: WorldTile,
    fetch: BankFetch,
    opts: FindOptions,
    avoid: &[AvoidRect],
) -> Option<RouteOutcome> {
    let opts = FindOptions {
        allow_bank_fetch: false,
        ..opts
    };
    let route = find_with_avoid(
        &world.collision,
        &world.graph,
        from,
        to,
        opts,
        &fetch.state,
        avoid,
    )
    .ok()?;
    Some(RouteOutcome::BankSession {
        pending: PendingBankFetch {
            steps: fetch.steps.into(),
            dest: to,
            opts,
            final_route: route.clone(),
            avoid: avoid.to_vec(),
            progress: StepProgress::default(),
        },
        route,
    })
}
