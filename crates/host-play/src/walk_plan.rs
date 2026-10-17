use std::any::Any;
#[cfg(test)]
use std::borrow::Cow;
use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;

use api::bank_memory::Origin;
use api::snapshot::WorldTile;
#[cfg(test)]
use nav::bank_fetch::fetchable_state;
use nav::bank_fetch::{plan_bank_fetch, planning_rows, BankFetch, BankStep};
use nav::contract::{LegView, NavLease, RoutePlanLease, RouteView};
#[cfg(test)]
use nav::router::{
    find_first_missing_item_reqs_with_avoid, find_first_with_avoid, find_first_with_fallback_avoid,
    missing_item_reqs, FallbackRoute,
};
use nav::router::{
    find_missing_item_reqs_with_avoid, find_with_avoid, AvoidRect, FindOptions, Route,
};
#[cfg(test)]
use nav::transport::TransportEdge;
use nav::world::NavWorld;
use nav::WorldState;

/// One route without expanding compact plans into legacy leg vectors.
#[expect(
    clippy::large_enum_variant,
    reason = "The compact branch shares existing backing and avoids another per-route heap allocation."
)]
#[derive(Clone)]
pub(crate) enum RoutePath {
    Legacy(Arc<Route>),
    Compact {
        plan: RoutePlanLease,
        lease: NavLease,
        _custody: Arc<dyn Any + Send + Sync>,
    },
}

impl RoutePath {
    pub(crate) fn legacy(route: Arc<Route>) -> Self {
        Self::Legacy(route)
    }

    pub(crate) fn compact(
        plan: RoutePlanLease,
        lease: NavLease,
        _custody: Arc<dyn Any + Send + Sync>,
    ) -> Self {
        Self::Compact {
            plan,
            lease,
            _custody,
        }
    }

    pub(crate) fn destination(&self) -> WorldTile {
        match self {
            Self::Legacy(route) => route.dest,
            Self::Compact { plan, .. } => plan.destination(),
        }
    }

    pub(crate) fn legacy_route(&self) -> Option<&Arc<Route>> {
        match self {
            Self::Legacy(route) => Some(route),
            Self::Compact { .. } => None,
        }
    }

    pub(crate) fn plan(&self) -> Option<&RoutePlanLease> {
        match self {
            Self::Legacy(_) => None,
            Self::Compact { plan, .. } => Some(plan),
        }
    }

    pub(crate) fn plan_for_follow(&self) -> Option<RoutePlanLease> {
        self.plan().cloned()
    }

    pub(crate) fn view(&self) -> Option<RouteView<'_>> {
        match self {
            Self::Legacy(_) => None,
            Self::Compact { plan, lease, .. } => plan.view(lease).ok(),
        }
    }

    pub(crate) fn projection(&self) -> crate::walk_map::RouteProjectionPath<'_> {
        match self {
            Self::Legacy(route) => crate::walk_map::RouteProjectionPath::Legacy(route),
            Self::Compact { .. } => crate::walk_map::RouteProjectionPath::Compact(
                self.view().expect("compact route keeps its bound lease"),
            ),
        }
    }

    pub(crate) fn first_tile(&self) -> Option<WorldTile> {
        match self {
            Self::Legacy(route) => match route.legs.first()? {
                nav::router::Leg::Walk { tiles } => tiles.first().copied(),
                nav::router::Leg::Transport { edge } => Some(edge.at),
            },
            Self::Compact { plan, lease, .. } => match plan.view(lease).ok()?.leg(0).ok()? {
                LegView::Walk(tiles) => tiles.first().copied(),
                LegView::Transport(edge) => Some(edge.anchor()),
            },
        }
    }
}

impl fmt::Debug for RoutePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Legacy(route) => formatter.debug_tuple("Legacy").field(route).finish(),
            Self::Compact { plan, .. } => formatter
                .debug_struct("Compact")
                .field("identity", &plan.identity())
                .field("revision", &plan.revision())
                .field("destination", &plan.destination())
                .finish(),
        }
    }
}

impl PartialEq for RoutePath {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Legacy(left), Self::Legacy(right)) => Arc::ptr_eq(left, right),
            (Self::Compact { plan: left, .. }, Self::Compact { plan: right, .. }) => {
                left.identity() == right.identity()
                    && left.revision() == right.revision()
                    && left.destination() == right.destination()
            }
            _ => false,
        }
    }
}

impl Eq for RoutePath {}
impl From<Arc<Route>> for RoutePath {
    fn from(route: Arc<Route>) -> Self {
        Self::legacy(route)
    }
}

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
    pub(crate) final_route: RoutePath,
    /// The walk's avoidance rectangles: the access sub-route keeps out of
    /// them as the walk does.
    pub avoid: Vec<AvoidRect>,
    /// The front step's pump budget and in-flight send.
    pub progress: StepProgress,
}

impl PendingBankFetch {
    /// Build a legacy-route fixture without exposing the private route backing.
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test_with_route(
        steps: VecDeque<BankStep>,
        dest: WorldTile,
        opts: FindOptions,
        final_route: Arc<Route>,
    ) -> Self {
        Self {
            steps,
            dest,
            opts,
            final_route: RoutePath::legacy(final_route),
            avoid: Vec::new(),
            progress: StepProgress::default(),
        }
    }

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
/// not re-send until the step's target changes. Wear ignores bank identity
/// because the bank close is part of its own interaction.
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
    /// Withdraw / Withdraw-X: the obj's backpack total and whether it is worn.
    Obj { id: i32, carried: i32, worn: bool },
    /// Wear: the inventory rows targeted by `Interactions::wear` and worn
    /// state, independent of the bank session it closes.
    Wear { id: i32, carried: i32, worn: bool },
}

#[expect(
    clippy::large_enum_variant,
    reason = "The route result retains both bank routes inline without an extra result allocation."
)]
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
                &world.graph,
                opts,
                avoid,
            )
            .ok()
        })
        .and_then(|fetch| session_route(world, from, to, fetch, opts, avoid))
        .unwrap_or(RouteOutcome::NoPath),
        Err(_) => RouteOutcome::NoPath,
    }
}

#[cfg(test)]
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

#[cfg(test)]
#[expect(
    clippy::large_enum_variant,
    reason = "The synchronous reference fixture retains its complete route outcome inline."
)]
/// What the search for a stand a session can reach established.
pub(super) enum StandFetch {
    /// A session to a stand, or a strict route to one past the strict
    /// search's budget.
    Outcome(RouteOutcome),
    /// No stand: the same search's answer for the radius tiles, if any.
    Tiles(Option<FallbackRoute>),
}

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
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
        &world.graph,
        opts,
        avoid,
    )
    .ok()?;
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
    let final_route = RoutePath::legacy(Arc::new(route.clone()));
    Some(RouteOutcome::BankSession {
        pending: PendingBankFetch {
            steps: fetch.steps.into(),
            dest: to,
            opts,
            final_route,
            avoid: avoid.to_vec(),
            progress: StepProgress::default(),
        },
        route,
    })
}
