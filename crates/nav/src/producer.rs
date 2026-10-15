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

use std::sync::Arc;

use crate::contract::{
    BoundZoneExempt, BoundZoneKey, CollisionView, ContractError, NavIdentity, NavLease, PackDigest,
    PackId, RouteView, WildernessView, ZoneTableView,
};
use crate::router::Route;
use crate::world::NavWorld;
use crate::zones::{ZoneExempt, ZoneKey};

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

/// A lease over `world` under its one identity. Cloning the lease clones
/// the `Arc` handle only. The borrowed producers below serve the same
/// identity without `Arc` traffic.
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
