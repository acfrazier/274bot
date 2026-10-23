//! Immutable identity-bound route plan and borrowed leg views. Does not
//! expose public router `Route`/`Leg` Vec/Box layouts or let script
//! construct/destructure storage.

use std::fmt;
use std::sync::Arc;

use api::identity::WorkIdentity;
use api::snapshot::WorldTile;
use api::work::InputRevision;

use crate::producer::OwnerResultReservation;
use crate::router::{CompactRoute, CompactRouteCursor, CompactRouteLeg, Leg, Route};

use super::transport::TransportView;
use super::{ContractError, NavLease, PackId};

/// Storage shared by route handles. It retains its immutable graph lease and
/// owner-result charge; it never clones transport edges.
struct RoutePlanStorage {
    route: CompactRoute,
    _charge: OwnerResultReservation,
}

/// Once-published plan ownership, shared by `Arc`. Identity-bound to the
/// originating pack and work request.
///
/// [`Clone`] clones the plan handle, not route storage. That is the
/// consumer justification (T5.1 publishes once; T1/T2 borrow the view).
#[derive(Clone)]
pub struct RoutePlanLease {
    pack: PackId,
    identity: WorkIdentity,
    revision: InputRevision,
    route: Arc<RoutePlanStorage>,
}

impl RoutePlanLease {
    /// Owner-boundary constructor for a compact route result.
    pub(crate) fn from_compact(
        pack: PackId,
        identity: WorkIdentity,
        revision: InputRevision,
        lease: NavLease,
        mut route: CompactRoute,
        charge: OwnerResultReservation,
    ) -> Self {
        route.retain_lease(lease);
        Self {
            pack,
            identity,
            revision,
            route: Arc::new(RoutePlanStorage {
                route,
                _charge: charge,
            }),
        }
    }
    pub(crate) fn compact_storage_bytes(route: &CompactRoute) -> Option<usize> {
        route
            .try_retained_bytes()?
            .checked_add(std::mem::size_of::<RoutePlanStorage>())?
            .checked_add(2 * std::mem::size_of::<usize>())
    }

    /// Originating pack.
    pub const fn pack(&self) -> PackId {
        self.pack
    }

    /// Work identity this plan answers.
    pub const fn identity(&self) -> WorkIdentity {
        self.identity
    }

    /// Input revision this plan was computed for.
    pub const fn revision(&self) -> InputRevision {
        self.revision
    }

    /// Exact destination tile.
    pub fn destination(&self) -> WorldTile {
        self.route.route.destination()
    }

    pub(crate) fn leg_count(&self) -> usize {
        self.route.route.leg_count()
    }

    pub(crate) fn retained_leg(&self, index: usize) -> Option<CompactRouteLeg<'_>> {
        let route = &self.route.route;
        route.leg_at(route.graph()?, index)
    }

    pub(crate) fn owned_leg(&self, index: usize) -> Option<Leg> {
        let route = &self.route.route;
        route.owned_leg(route.graph()?, index)
    }

    /// Borrow a view through `lease`. Refused when `lease` is another pack.
    pub fn view<'a>(&'a self, lease: &'a NavLease) -> Result<RouteView<'a>, ContractError> {
        if lease.identity().pack() != self.pack {
            return Err(ContractError::PackMismatch);
        }
        Ok(RouteView {
            pack: self.pack,
            source: RouteSource::Plan(&self.route),
        })
    }

    pub(crate) fn compact_route(&self) -> &CompactRoute {
        &self.route.route
    }
}

impl fmt::Debug for RoutePlanLease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RoutePlanLease")
            .field("pack", &self.pack)
            .field("identity", &self.identity)
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy)]
enum RouteSource<'a> {
    Borrowed(&'a Route),
    Plan(&'a RoutePlanStorage),
    Compact(
        &'a crate::router::CompactRoute,
        &'a crate::transport::TransportGraph,
    ),
}

impl<'a> RouteSource<'a> {
    fn route_facts(&self) -> (WorldTile, f64, usize) {
        match self {
            Self::Borrowed(route) => (route.dest, route.ticks, route.legs.len()),
            Self::Plan(plan) => (
                plan.route.destination(),
                plan.route.ticks(),
                plan.route.leg_count(),
            ),
            Self::Compact(route, _) => (route.destination(), route.ticks(), route.leg_count()),
        }
    }

    fn compact(
        self,
    ) -> Option<(
        &'a crate::router::CompactRoute,
        &'a crate::transport::TransportGraph,
    )> {
        match self {
            Self::Plan(plan) => Some((
                &plan.route,
                plan.route
                    .graph()
                    .expect("published route retains its nav lease"),
            )),
            Self::Compact(route, graph) => Some((route, graph)),
            Self::Borrowed(_) => None,
        }
    }

    fn legacy(self) -> Option<&'a Route> {
        match self {
            Self::Borrowed(route) => Some(route),
            Self::Plan(_) | Self::Compact(_, _) => None,
        }
    }
}

/// Borrowed route: destination, exact f64 tick cost, bounded legs. Served
/// through [`RoutePlanLease::view`], from a live compact search completion
/// (`crate::router::SearchCompletion::route_view`), or from an owner's retained
/// legacy route (`crate::producer::route_view`).
#[derive(Clone, Copy)]
pub struct RouteView<'a> {
    pack: PackId,
    source: RouteSource<'a>,
}

impl<'a> RouteView<'a> {
    /// A view over a route the owner boundary retains for `pack`
    /// (`crate::producer::route_view`); the view cannot outlive it.
    pub(crate) const fn borrowed(pack: PackId, route: &'a Route) -> Self {
        Self {
            pack,
            source: RouteSource::Borrowed(route),
        }
    }

    /// A borrowed view over a compact result kept by the cooperative cursor
    /// owner until assessment and final publication complete.
    pub(crate) const fn borrowed_compact(
        pack: PackId,
        route: &'a crate::router::CompactRoute,
        graph: &'a crate::transport::TransportGraph,
    ) -> Self {
        Self {
            pack,
            source: RouteSource::Compact(route, graph),
        }
    }

    /// Originating pack.
    pub const fn pack(self) -> PackId {
        self.pack
    }

    /// Destination tile.
    pub fn destination(self) -> WorldTile {
        self.source.route_facts().0
    }

    /// Total tick cost, retaining f64 half-ticks. Never truncated.
    pub fn ticks(self) -> f64 {
        self.source.route_facts().1
    }

    /// Number of legs.
    pub fn leg_count(self) -> usize {
        self.source.route_facts().2
    }

    /// Bounded leg access. Out of range is [`ContractError::NoSuchLeg`].
    pub fn leg(self, index: usize) -> Result<LegView<'a>, ContractError> {
        if let Some(route) = self.source.legacy() {
            return match route.legs.get(index) {
                Some(Leg::Walk { tiles }) => Ok(LegView::Walk(tiles.as_slice())),
                Some(Leg::Transport { edge }) => {
                    Ok(LegView::Transport(TransportView::from_edge(edge)))
                }
                None => Err(ContractError::NoSuchLeg),
            };
        }
        let (route, graph) = self.source.compact().expect("route source is valid");
        match route.leg_at(graph, index) {
            Some(CompactRouteLeg::Walk(tiles)) => Ok(LegView::Walk(tiles)),
            Some(CompactRouteLeg::Transport(step)) => {
                Ok(LegView::Transport(TransportView::from_compact_step(step)))
            }
            None => Err(ContractError::NoSuchLeg),
        }
    }

    /// Borrowed iterator over legs. Allocates/copies nothing.
    pub fn legs(self) -> impl Iterator<Item = LegView<'a>> + 'a {
        if let Some(route) = self.source.legacy() {
            RouteLegIter::Legacy(route.legs.iter())
        } else {
            let (route, graph) = self.source.compact().expect("route source is valid");
            RouteLegIter::Compact(route.cursor(graph))
        }
    }
}

enum RouteLegIter<'a> {
    Legacy(std::slice::Iter<'a, Leg>),
    Compact(CompactRouteCursor<'a>),
}

impl<'a> Iterator for RouteLegIter<'a> {
    type Item = LegView<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Legacy(legs) => legs.next().map(|leg| match leg {
                Leg::Walk { tiles } => LegView::Walk(tiles.as_slice()),
                Leg::Transport { edge } => LegView::Transport(TransportView::from_edge(edge)),
            }),
            Self::Compact(legs) => legs.next_leg().map(|leg| match leg {
                CompactRouteLeg::Walk(tiles) => LegView::Walk(tiles),
                CompactRouteLeg::Transport(step) => {
                    LegView::Transport(TransportView::from_compact_step(step))
                }
            }),
        }
    }
}

impl fmt::Debug for RouteView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RouteView")
            .field("pack", &self.pack)
            .field("dest", &self.source.route_facts().0)
            .field("ticks", &self.source.route_facts().1)
            .field("leg_count", &self.source.route_facts().2)
            .finish()
    }
}

/// One borrowed leg: walk tiles or transport facts.
#[derive(Clone, Copy, Debug)]
pub enum LegView<'a> {
    /// Consecutive walk tiles, borrowed from the plan.
    Walk(&'a [WorldTile]),
    /// One transport hop.
    Transport(TransportView<'a>),
}

impl<'a> LegView<'a> {
    /// Walk tiles, if this is a walk leg.
    pub const fn walk_tiles(self) -> Option<&'a [WorldTile]> {
        match self {
            Self::Walk(tiles) => Some(tiles),
            Self::Transport(_) => None,
        }
    }

    /// Transport facts, if this is a transport leg.
    pub const fn transport(self) -> Option<TransportView<'a>> {
        match self {
            Self::Walk(_) => None,
            Self::Transport(view) => Some(view),
        }
    }
}
