//! Owner-boundary producers for the [`crate::contract`] facade (T5.1,
//! contract §21 §5.3: "host-play producers manufacture the views/leases at
//! the owner boundary; script does not reopen pack files").
//!
//! Host-play manufactures leases, views and bound exemptions here from the
//! storage it owns: the loaded [`NavWorld`], the router's retained
//! [`Route`] and a request's raw [`ZoneExempt`]. Script imports
//! `nav::contract` alone; the architecture checker classes this module as
//! an implementation path, so a production script import of it is a
//! registered violation.
//!
//! One loaded world carries **one** facade identity. The host's pack loader
//! records the verified pack digest with [`bind`]; [`NavWorld::from_bytes`]
//! records the bytes' digest itself. The identity is minted on the first
//! view served and shared by every later lease and view of that world, so
//! a plan, bound key or exemption produced from one view is accepted by
//! every other view of the same world and refused by any other world
//! ([`ContractError::PackMismatch`]). A synthetic world (`from_parts`,
//! `from_grid`) that reaches a view without a bind carries the empty-content
//! digest: it is a fixture, not a loaded pack, and its [`PackId`] still
//! distinguishes it. [`crate::contract::NavLease::wrap`] keeps F0's
//! semantics (a fresh identity per call) and never touches this cache.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};

use api::identity::WorkIdentity;
use api::snapshot::WorldTile;
use api::work::InputRevision;

use crate::contract::{
    BoundZoneExempt, BoundZoneKey, CollisionView, ContractError, NavIdentity, NavLease, PackDigest,
    PackId, RoutePlanLease, RouteView, WildernessView, ZoneTableView,
};
use crate::router::{
    CompactRoute, CompactRouteCursor, CompactRouteLeg, CompactTransportStep, Route,
};
use crate::world::NavWorld;
use crate::zones::{ZoneExempt, ZoneKey};

/// Process-wide cap for caller-held route, inspection, and bank results.
pub const OWNER_RESULT_CAPACITY_BYTES: usize = 256 * 1024 * 1024;
/// A single ordinary result larger than this needs a new policy ruling.
pub const OWNER_RESULT_MAX_BYTES: usize = 64 * 1024 * 1024;

static OWNER_RESULT_LEDGER: LazyLock<OwnerResultLedger> = LazyLock::new(|| {
    OwnerResultLedger::with_limits(OWNER_RESULT_CAPACITY_BYTES, OWNER_RESULT_MAX_BYTES)
});

/// Shared bounded accounting for results after WorkAdmission terminal custody.
#[derive(Clone)]
pub struct OwnerResultLedger {
    inner: Arc<OwnerResultLedgerInner>,
}

struct OwnerResultLedgerInner {
    capacity_bytes: usize,
    max_result_bytes: usize,
    used_bytes: AtomicUsize,
}

impl OwnerResultLedger {
    /// The one process-wide route/inspection/bank result ledger.
    pub fn shared() -> &'static Self {
        &OWNER_RESULT_LEDGER
    }

    /// An independently bounded ledger for deterministic owner tests.
    pub fn with_limits(capacity_bytes: usize, max_result_bytes: usize) -> Self {
        Self {
            inner: Arc::new(OwnerResultLedgerInner {
                capacity_bytes,
                max_result_bytes,
                used_bytes: AtomicUsize::new(0),
            }),
        }
    }

    /// Aggregate capacity in bytes.
    pub fn capacity_bytes(&self) -> usize {
        self.inner.capacity_bytes
    }

    /// Maximum bytes admitted for one result.
    pub fn max_result_bytes(&self) -> usize {
        self.inner.max_result_bytes
    }

    /// Current caller-held result bytes, including published leases.
    pub fn used_bytes(&self) -> usize {
        self.inner.used_bytes.load(Ordering::Relaxed)
    }
    /// Reserve exact retained bytes before consuming the WorkAdmission
    /// terminal. `Full` is back-pressure; retry on a later ordinary demand
    /// turn. Dropping the reservation releases its charge.
    pub fn try_reserve_bytes(
        &self,
        bytes: usize,
    ) -> Result<OwnerResultReservation, OwnerResultRefusal> {
        if bytes > self.inner.max_result_bytes {
            return Err(OwnerResultRefusal::TooLarge {
                requested: bytes,
                maximum: self.inner.max_result_bytes,
            });
        }
        let used = &self.inner.used_bytes;
        let mut current = used.load(Ordering::Relaxed);
        loop {
            let Some(next) = current.checked_add(bytes) else {
                return Err(OwnerResultRefusal::Full {
                    requested: bytes,
                    used: current,
                    capacity: self.inner.capacity_bytes,
                });
            };
            if next > self.inner.capacity_bytes {
                return Err(OwnerResultRefusal::Full {
                    requested: bytes,
                    used: current,
                    capacity: self.inner.capacity_bytes,
                });
            }
            match used.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => break,
                Err(observed) => current = observed,
            }
        }
        Ok(OwnerResultReservation {
            charge: Arc::new(OwnerResultCharge {
                ledger: Arc::clone(&self.inner),
                bytes,
            }),
        })
    }

    /// Exact caller-held bytes for an `Arc<OwnerResult<T>>`; `retained` is
    /// the sum of T's separately allocated buffers.
    pub fn owner_result_bytes<T>(retained: usize) -> Option<usize> {
        let (allocation, _) = std::alloc::Layout::new::<[AtomicUsize; 2]>()
            .extend(std::alloc::Layout::new::<OwnerResult<T>>())
            .ok()?;
        retained
            .checked_add(allocation.pad_to_align().size())?
            .checked_add(OwnerResultReservation::storage_bytes())
    }
    /// Reserve a compact route plan's exact retained bytes before consuming
    /// its WorkAdmission terminal.
    pub fn reserve_compact_route(
        &self,
        route: &CompactRoute,
    ) -> Result<OwnerResultReservation, OwnerResultRefusal> {
        let bytes = compact_route_result_bytes(route).ok_or(OwnerResultRefusal::SizeOverflow)?;
        self.try_reserve_bytes(bytes)
    }
}

/// Why a caller-held result could not take custody.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum OwnerResultRefusal {
    /// The aggregate ledger is full; the WorkAdmission terminal remains
    /// charged until a later ordinary demand turn can retry.
    #[error("owner result ledger full: {used}+{requested}>{capacity} bytes")]
    Full {
        /// Requested retained bytes.
        requested: usize,
        /// Currently retained bytes.
        used: usize,
        /// Aggregate capacity.
        capacity: usize,
    },
    /// A single result exceeds the approved per-result ceiling.
    #[error("owner result {requested} bytes exceeds the {maximum}-byte ceiling")]
    TooLarge {
        /// Requested retained bytes.
        requested: usize,
        /// Largest admitted result.
        maximum: usize,
    },
    /// Checked result-size arithmetic overflowed; no reservation was made.
    #[error("owner result retained-byte calculation overflowed")]
    SizeOverflow,
}

struct OwnerResultCharge {
    ledger: Arc<OwnerResultLedgerInner>,
    bytes: usize,
}

// Final-owner release is lock-free: published route results may be retired
// while the slot's NavBot mutex is held, with no WorkService lock inversion.
impl Drop for OwnerResultCharge {
    fn drop(&mut self) {
        let previous = self
            .ledger
            .used_bytes
            .fetch_sub(self.bytes, Ordering::Relaxed);
        debug_assert!(previous >= self.bytes, "owner result ledger underflow");
    }
}

/// Shared accounting token. Clones share one reservation; they do not add
/// capacity. It is embedded in every alias whose bytes remain live.
#[derive(Clone)]
pub struct OwnerResultReservation {
    charge: Arc<OwnerResultCharge>,
}

impl OwnerResultReservation {
    /// Bytes accounted by this shared reservation.
    pub fn bytes(&self) -> usize {
        self.charge.bytes
    }
    /// Bytes for this reservation's shared Arc allocation.
    pub fn storage_bytes() -> usize {
        std::mem::size_of::<OwnerResultCharge>() + 2 * std::mem::size_of::<usize>()
    }

    /// Keep a non-route result in a final-Arc-released ledger lease.
    pub fn hold<T>(self, value: T) -> Arc<OwnerResult<T>> {
        Arc::new(OwnerResult {
            value,
            reservation: self,
        })
    }
}

/// A result whose shared reservation is released only with its final Arc.
pub struct OwnerResult<T> {
    value: T,
    reservation: OwnerResultReservation,
}

impl<T> OwnerResult<T> {
    /// Borrow the result without releasing its owner-ledger charge.
    pub const fn value(&self) -> &T {
        &self.value
    }

    /// Bytes retained by this result's shared reservation.
    pub fn charged_bytes(&self) -> usize {
        self.reservation.bytes()
    }
}

impl<T> std::ops::Deref for OwnerResult<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for OwnerResult<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("OwnerResult").field(&self.value).finish()
    }
}

/// Record the loaded pack's content digest on `world` and mint its
/// identity. The first bind wins; a later bind with the same digest is a
/// no-op and a different digest is refused with
/// [`ContractError::PackMismatch`] (the caller is naming another pack's
/// bytes).
pub fn bind(world: &NavWorld, content: PackDigest) -> Result<NavIdentity, ContractError> {
    let recorded = world.content.get_or_init(|| content);
    if *recorded != content {
        return Err(ContractError::PackMismatch);
    }
    let _ = world.search_graph_envelope();
    identity(world)
}

/// The world's one facade identity, minted on first use.
pub fn identity(world: &NavWorld) -> Result<NavIdentity, ContractError> {
    if let Some(identity) = world.lease_identity.get() {
        return Ok(*identity);
    }
    let content = *world.content.get_or_init(|| PackDigest::of(&[]));
    let minted = NavIdentity::mint(content)?;
    // A racing first use may have won; its identity is the world's, and the
    // losing mint is never reused.
    Ok(*world.lease_identity.get_or_init(|| minted))
}

/// A lease over `world` under its one identity. Cloning copies the `Arc`
/// handle and checked scalar envelope. Production `bind` prepares the
/// envelope before publishing the loaded world; synthetic worlds initialize
/// it on their first lease.
pub fn lease(world: &Arc<NavWorld>) -> Result<NavLease, ContractError> {
    Ok(NavLease::with_identity(Arc::clone(world), identity(world)?))
}

/// Borrowed collision view over the world's packed bake (the per-tick
/// producer).
pub fn collision_view(world: &NavWorld) -> Result<CollisionView<'_>, ContractError> {
    Ok(CollisionView::new(
        identity(world)?.pack(),
        &world.collision,
    ))
}

/// Borrowed zone table, or `Ok(None)` on a legacy/synthetic world with no
/// packed zones.
pub fn zones_view(world: &NavWorld) -> Result<Option<ZoneTableView<'_>>, ContractError> {
    let pack = identity(world)?.pack();
    Ok(world
        .graph
        .zones
        .as_ref()
        .map(|table| ZoneTableView::new(pack, table)))
}

/// Borrowed wilderness membership/level facts.
pub fn wilderness_view(world: &NavWorld) -> Result<WildernessView<'_>, ContractError> {
    identity(world)?;
    Ok(WildernessView::new(&world.graph.wilderness))
}

/// What a borrowed route view is served under: the loaded world (its one
/// identity) or a lease minted for it. Both name the same pack.
pub trait ViewSource {
    /// The pack identity the view is bound to.
    fn pack(&self) -> Result<PackId, ContractError>;
}

impl ViewSource for NavWorld {
    fn pack(&self) -> Result<PackId, ContractError> {
        identity(self).map(NavIdentity::pack)
    }
}

impl ViewSource for NavLease {
    fn pack(&self) -> Result<PackId, ContractError> {
        Ok(self.identity().pack())
    }
}

/// A [`RouteView`] over a route the owner retains, under `source`'s
/// identity. The view borrows the route (no copy, no `Arc` traffic) and
/// is accepted only against that identity's other views. Plan publication
/// under a work identity is T5.W(route).
pub fn route_view<'a>(
    source: &impl ViewSource,
    route: &'a Route,
) -> Result<RouteView<'a>, ContractError> {
    Ok(RouteView::borrowed(source.pack()?, route))
}

/// Borrow an unpublished compact result for cooperative assessment.
///
/// Both the route buffers and their immutable graph remain borrowed. No plan
/// is published and no storage is copied. An unbound or foreign-pack route is
/// refused before its compact edge indexes can be resolved.
pub fn compact_route_view<'a>(
    lease: &'a NavLease,
    route: &'a CompactRoute,
) -> Result<RouteView<'a>, ContractError> {
    let pack = lease.identity().pack();
    if route.pack() != Some(pack) {
        return Err(ContractError::PackMismatch);
    }
    Ok(RouteView::borrowed_compact(pack, route, lease.graph()))
}

/// One borrowed compact route leg for the host owner boundary.
#[derive(Clone, Copy, Debug)]
pub enum RoutePlanLeg<'a> {
    /// Consecutive walk tiles.
    Walk(&'a [WorldTile]),
    /// One packed transport or synthetic essence-return hop.
    Transport(RouteTransportStep<'a>),
}

/// A transport step with stable compact edge identity and actual landing.
pub type RouteTransportStep<'a> = CompactTransportStep<'a>;

/// Allocation-free owner-boundary iteration over one route plan.
pub struct RoutePlanCursor<'a> {
    cursor: CompactRouteCursor<'a>,
}

impl<'a> RoutePlanCursor<'a> {
    /// Advance exactly one stored leg without allocating.
    pub fn next_leg(&mut self) -> Option<RoutePlanLeg<'a>> {
        self.cursor.next_leg().map(|leg| match leg {
            CompactRouteLeg::Walk(tiles) => RoutePlanLeg::Walk(tiles),
            CompactRouteLeg::Transport(step) => RoutePlanLeg::Transport(step),
        })
    }
}

/// Borrow a plan's legs at the producer boundary. The cursor retains neither
/// the plan nor any per-hop copy; its borrow keeps the plan's graph lease live.
pub fn route_plan_cursor(plan: &RoutePlanLease) -> Result<RoutePlanCursor<'_>, ContractError> {
    let route = plan.compact_route();
    let graph = route
        .graph()
        .expect("published route retains its nav lease");
    Ok(RoutePlanCursor {
        cursor: route.cursor(graph),
    })
}

/// Bytes retained by a compact route plan, excluding the shared reservation
/// allocation. Useful when this plan is nested in a larger owner result.
pub fn compact_route_storage_bytes(route: &CompactRoute) -> Option<usize> {
    RoutePlanLease::compact_storage_bytes(route)
}

/// Complete bytes retained by a standalone compact route plan.
pub fn compact_route_result_bytes(route: &CompactRoute) -> Option<usize> {
    compact_route_storage_bytes(route)?
        .checked_add(std::mem::size_of::<RoutePlanLease>())?
        .checked_add(OwnerResultReservation::storage_bytes())
}

/// Publish a compact search result against its immutable pack lease and the
/// reservation acquired before consuming the WorkAdmission terminal.
pub fn publish_compact_route(
    lease: &NavLease,
    identity: WorkIdentity,
    revision: InputRevision,
    route: CompactRoute,
    reservation: OwnerResultReservation,
) -> RoutePlanLease {
    let required =
        compact_route_result_bytes(&route).expect("compact route result size must fit usize");
    assert!(
        reservation.bytes() >= required,
        "compact route result must be charged before publication"
    );
    RoutePlanLease::from_compact(
        lease.identity().pack(),
        identity,
        revision,
        lease.clone(),
        route,
        reservation,
    )
}

/// Bind a request's raw exemption to the pack behind `zones`. Every named
/// key must be a row or group of that pack ([`ContractError::UnknownKey`]
/// otherwise); `all()` binds without touching the table.
pub fn bind_exempt(
    zones: ZoneTableView<'_>,
    exempt: ZoneExempt,
) -> Result<BoundZoneExempt, ContractError> {
    if exempt.is_all() {
        return Ok(zones.exempt_all());
    }
    for key in exempt.zone_keys() {
        zones.bind(key)?;
    }
    Ok(BoundZoneExempt::bound(zones.pack(), exempt))
}

/// Bind a raw zone or group key the owner holds (a router witness) to the
/// pack behind `zones`. A key naming no row or group of that pack is
/// [`ContractError::UnknownKey`].
pub fn bind_key(zones: ZoneTableView<'_>, key: ZoneKey) -> Result<BoundZoneKey, ContractError> {
    zones.bind(key)
}

/// The member zone keys of a bound group, bound to the same pack, in pack
/// order, for an owner-boundary check that judges a group by every member.
/// A zone key or an unknown group is [`ContractError::UnknownKey`]; a key
/// from another pack is [`ContractError::PackMismatch`].
pub fn group_members<'a>(
    zones: ZoneTableView<'a>,
    group: BoundZoneKey,
) -> Result<impl Iterator<Item = BoundZoneKey> + 'a, ContractError> {
    zones.members(group)
}

#[cfg(test)]
#[path = "producer_tests.rs"]
mod tests;
