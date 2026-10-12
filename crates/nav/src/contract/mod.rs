//! Semantic nav facade (F0 item 4: contract §23 §1.1, D11).
//!
//! Views and leases wrap the current backend (`arrival`, `collision`,
//! `router`, `transport`, `zones`) without exposing storage layouts.
//! A re-export of [`crate::collision::WorldCollision`], [`crate::router::Route`],
//! [`crate::transport::TransportEdge`] or [`crate::zones::ZoneTable`] is not
//! this surface. Production import migration is T5.1 alone.
//!
//! Small semantic values stay stable and are re-exported: [`ArrivalKind`],
//! [`ZoneKey`], [`ZoneClass`], [`ZoneExempt`], [`TransportKind`],
//! [`DoorDir`], [`AvoidRect`], [`MAX_ZONE_KEYS`], [`AVOID_CATALOG_IDS`].
//! Coordinates are [`api::snapshot::WorldTile`].

use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::world::NavWorld;

mod arrival;
mod collision;
mod envelope;
mod route;
mod transport;
mod zones;

pub use arrival::ArrivalSpec;
pub use collision::CollisionView;
pub use envelope::{RouteFamilyError, RouteFamilyResult, RoutePermissions, RouteRequest};
pub use route::{LegView, RoutePlanLease, RouteView};
pub use transport::TransportView;
pub use zones::{BoundZoneKey, WildernessView, ZoneKindView, ZoneTableView, ZoneView};

pub use crate::arrival::ArrivalKind;
pub use crate::pack::{FORMAT_ID, VERSION};
pub use crate::router::AvoidRect;
pub use crate::transport::{DoorDir, TransportKind};
pub use crate::zones::{TooMany, ZoneClass, ZoneExempt, ZoneKey, AVOID_CATALOG_IDS, MAX_ZONE_KEYS};

/// Opaque identity of one immutable pack lease. A numeric zone or route
/// index is meaningless without it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PackId(NonZeroU64);

impl PackId {
    fn mint() -> Result<Self, ContractError> {
        static LAST: AtomicU64 = AtomicU64::new(0);
        match LAST.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |last| {
            last.checked_add(1)
        }) {
            Ok(previous) => NonZeroU64::new(previous.wrapping_add(1))
                .map(Self)
                .ok_or(ContractError::PackIdExhausted),
            Err(_) => Err(ContractError::PackIdExhausted),
        }
    }

    /// The raw lease id. Not a table index.
    pub const fn get(self) -> NonZeroU64 {
        self.0
    }
}

/// Nav revision/content identity carried by every view and plan lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NavIdentity {
    pack: PackId,
    format: &'static str,
}

impl NavIdentity {
    /// The originating pack lease.
    pub const fn pack(self) -> PackId {
        self.pack
    }

    /// Pack format identity (`274V16` today).
    pub const fn format(self) -> &'static str {
        self.format
    }
}

/// Why a facade query was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ContractError {
    /// A view, plan or bound key was used against another pack.
    #[error("nav view or index belongs to another pack")]
    PackMismatch,
    /// The zone or group key is not present in this pack.
    #[error("zone or group key is unknown in this pack")]
    UnknownKey,
    /// The requested leg does not exist on this plan.
    #[error("route leg index is out of range")]
    NoSuchLeg,
    /// Pack lease ids exhausted; never reuse.
    #[error("nav pack lease identity space exhausted")]
    PackIdExhausted,
}

/// Immutable pack lease. Concrete views borrow it; they cannot outlive it
/// or index a different pack. Not a per-tick clone and not a heap-allocated
/// trait object.
#[derive(Clone)]
pub struct NavLease {
    identity: NavIdentity,
    world: Arc<NavWorld>,
}

impl NavLease {
    /// Wrap a loaded world as an immutable lease. T5.1's host producer
    /// constructs this at the owner boundary; script does not reopen packs.
    pub fn wrap(world: Arc<NavWorld>) -> Result<Self, ContractError> {
        Ok(Self {
            identity: NavIdentity {
                pack: PackId::mint()?,
                format: FORMAT_ID,
            },
            world,
        })
    }

    /// Pack/format identity.
    pub const fn identity(&self) -> NavIdentity {
        self.identity
    }

    /// Borrowed collision view over this lease's bake.
    pub fn collision(&self) -> CollisionView<'_> {
        CollisionView::new(self.identity.pack, &self.world.collision)
    }

    /// Borrowed zone table, or `None` on a legacy/synthetic world with no
    /// packed zones (unknown/refusal).
    pub fn zones(&self) -> Option<ZoneTableView<'_>> {
        self.world
            .graph
            .zones
            .as_ref()
            .map(|table| ZoneTableView::new(self.identity.pack, table))
    }

    /// Borrowed wilderness membership/level facts.
    pub fn wilderness(&self) -> WildernessView<'_> {
        WildernessView::new(&self.world.graph.wilderness)
    }

    /// Publish an immutable, identity-bound plan from a backend [`crate::router::Route`].
    /// Owner-boundary constructor for T5.1; iterating the lease does not
    /// expose `Route`/`Leg` storage.
    pub fn publish_route(
        &self,
        identity: api::identity::WorkIdentity,
        revision: api::work::InputRevision,
        route: crate::router::Route,
    ) -> RoutePlanLease {
        RoutePlanLease::from_backend(self.identity.pack, identity, revision, route)
    }
}

impl std::fmt::Debug for NavLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NavLease")
            .field("identity", &self.identity)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "contract_tests.rs"]
mod tests;
