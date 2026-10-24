use std::any::Any;
use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;

use api::bank_memory::Origin;
use api::snapshot::WorldTile;
use nav_planner::bank_fetch::{plan_bank_fetch, planning_rows, BankFetch, BankStep};
use nav_planner::contract::{LegView, NavLease, RoutePlanLease, RouteView};
use nav_planner::router::{
    find_missing_item_reqs_with_avoid, find_with_avoid, AvoidRect, FindOptions, Route,
};
use nav_planner::world::NavWorld;
use nav_planner::WorldState;

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
                nav_planner::router::Leg::Walk { tiles } => tiles.first().copied(),
                nav_planner::router::Leg::Transport { edge } => Some(edge.at),
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
