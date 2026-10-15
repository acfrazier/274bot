//! Immutable identity-bound route plan and borrowed leg views. Does not
//! expose public router `Route`/`Leg` Vec/Box layouts or let script
//! construct/destructure storage.

use std::fmt;
use std::sync::Arc;

use api::identity::WorkIdentity;
use api::snapshot::WorldTile;
use api::work::InputRevision;

use crate::router::{Leg, Route};

use super::transport::TransportView;
use super::{ContractError, NavLease, PackId};

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
    route: Arc<Route>,
}

impl RoutePlanLease {
    /// Owner-boundary constructor; test-only until T5.1 adds the producer.
    #[cfg(test)]
    pub(super) fn from_backend(
        pack: PackId,
        identity: WorkIdentity,
        revision: InputRevision,
        route: Route,
    ) -> Self {
        Self {
            pack,
            identity,
            revision,
            route: Arc::new(route),
        }
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

    /// Borrow a view through `lease`. Refused when `lease` is another pack.
    pub fn view<'a>(&'a self, lease: &'a NavLease) -> Result<RouteView<'a>, ContractError> {
        if lease.identity().pack() != self.pack {
            return Err(ContractError::PackMismatch);
        }
        Ok(RouteView {
            pack: self.pack,
            route: &self.route,
        })
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

/// Borrowed route: destination, exact f64 tick cost, bounded legs. Served
/// through [`RoutePlanLease::view`], or borrowed from the owner's retained
/// route by the owner boundary (`crate::producer::route_view`).
#[derive(Clone, Copy)]
pub struct RouteView<'a> {
    pack: PackId,
    route: &'a Route,
}

impl<'a> RouteView<'a> {
    /// A view over a route the owner boundary retains for `pack`
    /// (`crate::producer::route_view`); the view cannot outlive it.
    pub(crate) const fn borrowed(pack: PackId, route: &'a Route) -> Self {
        Self { pack, route }
    }

    /// Originating pack.
    pub const fn pack(self) -> PackId {
        self.pack
    }

    /// Destination tile.
    pub fn destination(self) -> WorldTile {
        self.route.dest
    }

    /// Total tick cost, retaining f64 half-ticks. Never truncated.
    pub fn ticks(self) -> f64 {
        self.route.ticks
    }

    /// Number of legs.
    pub fn leg_count(self) -> usize {
        self.route.legs.len()
    }

    /// Bounded leg access. Out of range is [`ContractError::NoSuchLeg`].
    pub fn leg(self, index: usize) -> Result<LegView<'a>, ContractError> {
        match self.route.legs.get(index) {
            Some(Leg::Walk { tiles }) => Ok(LegView::Walk(tiles.as_slice())),
            Some(Leg::Transport { edge }) => Ok(LegView::Transport(TransportView::from_edge(edge))),
            None => Err(ContractError::NoSuchLeg),
        }
    }

    /// Borrowed iterator over legs. Allocates/copies nothing.
    pub fn legs(self) -> impl Iterator<Item = LegView<'a>> + 'a {
        self.route.legs.iter().map(|leg| match leg {
            Leg::Walk { tiles } => LegView::Walk(tiles.as_slice()),
            Leg::Transport { edge } => LegView::Transport(TransportView::from_edge(edge)),
        })
    }
}

impl fmt::Debug for RouteView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RouteView")
            .field("pack", &self.pack)
            .field("dest", &self.route.dest)
            .field("ticks", &self.route.ticks)
            .field("leg_count", &self.route.legs.len())
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
