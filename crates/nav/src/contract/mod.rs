//! Semantic nav facade (F0 item 4: contract §23 §1.1, D11).
//!
//! Views and leases wrap the current backend (`arrival`, `collision`,
//! `router`, `transport`, `zones`) without exposing storage layouts.
//! A re-export of [`crate::collision::WorldCollision`], [`crate::router::Route`],
//! [`crate::transport::TransportEdge`] or [`crate::zones::ZoneTable`] is not
//! this surface. Production import migration is T5.1 alone.
//!
//! Small semantic values stay stable and are re-exported: [`ArrivalKind`],
//! [`ZoneKey`], [`ZoneClass`], [`TransportKind`], [`DoorDir`], [`AvoidRect`],
//! [`MAX_ZONE_KEYS`], [`AVOID_CATALOG_IDS`]. Zone exemptions are pack-bound
//! [`BoundZoneExempt`] values; raw [`crate::zones::ZoneExempt`] is not part
//! of this surface. Coordinates are [`api::snapshot::WorldTile`].

use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::world::NavWorld;

mod arrival;
mod collision;
mod envelope;
mod route;
mod transport;
mod zones;

pub use arrival::ArrivalSpec;
pub use collision::CollisionView;
pub use envelope::{
    RouteFamilyError, RouteFamilyResult, RoutePermissions, RoutePhase, RouteRequest, RouteService,
};
pub use route::{LegView, RoutePlanLease, RouteView};
pub use transport::TransportView;
pub use zones::{
    BoundZoneExempt, BoundZoneKey, WildernessView, ZoneKindView, ZoneTableView, ZoneView,
};

pub use crate::arrival::ArrivalKind;
pub use crate::pack::{FORMAT_ID, VERSION};
pub use crate::router::AvoidRect;
pub use crate::transport::{DoorDir, TransportKind};
pub use crate::zones::{ZoneClass, ZoneKey, AVOID_CATALOG_IDS, MAX_ZONE_KEYS};

/// SHA-256 digest of pack content. Two leases of the same bytes share this
/// identity and still have distinct [`PackId`]s.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PackDigest([u8; 32]);

impl PackDigest {
    /// Wrap an already-computed 32-byte digest (the owner-boundary
    /// `nav_sha256` / `nav::pack` hash).
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// The 32 content bytes.
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }

    /// Digest `bytes` with SHA-256.
    pub fn of(bytes: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let digest = hasher.finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&digest);
        Self(bytes)
    }
}

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
            Ok(previous) => NonZeroU64::new(previous + 1)
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

/// Nav revision/content identity carried by every view and plan lease:
/// the lease id, the content digest, and the pack format/version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NavIdentity {
    pack: PackId,
    content: PackDigest,
    format: &'static str,
    version: u8,
}

impl NavIdentity {
    /// The originating pack lease. Distinct even when content matches.
    pub const fn pack(self) -> PackId {
        self.pack
    }

    /// Content digest shared by every lease of the same bytes.
    pub const fn content(self) -> PackDigest {
        self.content
    }

    /// Pack format identity (`274V16` today).
    pub const fn format(self) -> &'static str {
        self.format
    }

    /// Pack format version byte (`16` today).
    pub const fn version(self) -> u8 {
        self.version
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
    /// Area arrival cannot name a loc origin (`native/walk.rs`: `(Area, Some(loc))`).
    #[error("area arrival cannot carry a loc identity")]
    AreaWithLoc,
    /// More than [`MAX_ZONE_KEYS`] named exemptions.
    #[error("more than {MAX_ZONE_KEYS} zone exemptions")]
    TooManyExemptions,
}

/// Immutable pack lease. Concrete views borrow it; they cannot outlive it
/// or index a different pack. Not a per-tick clone and not a heap-allocated
/// trait object.
///
/// [`Clone`] clones the `Arc` handle (the bake is not copied). T5.1's host
/// producer holds one handle per loaded pack and shares it; that is the
/// consumer justification.
#[derive(Clone)]
pub struct NavLease {
    identity: NavIdentity,
    world: Arc<NavWorld>,
}

impl NavLease {
    /// Wrap a loaded world as an immutable lease with its content digest.
    /// T5.1's host producer constructs this at the owner boundary; script
    /// does not reopen packs.
    pub fn wrap(world: Arc<NavWorld>, content: PackDigest) -> Result<Self, ContractError> {
        Ok(Self {
            identity: NavIdentity {
                pack: PackId::mint()?,
                content,
                format: FORMAT_ID,
                version: VERSION,
            },
            world,
        })
    }

    /// Pack/format/content identity.
    pub const fn identity(&self) -> NavIdentity {
        self.identity
    }

    /// Borrowed collision view over this lease's packed bake.
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

    /// Publish an immutable, identity-bound plan from a backend route.
    /// Owner-boundary constructor; iterating the lease does not expose
    /// `Route`/`Leg` storage. Not part of the frozen public facade
    /// (hidden storage must not appear in a public signature).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn publish_route(
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
