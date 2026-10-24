//! Service envelope: permissions/admission, typed route terminals, and the
//! [`RouteService`] binding of [`BoundRouteRequest`] onto
//! [`api::work::WorkFamily::Route`]. Concrete views borrow the originating
//! [`super::NavLease`]; a request is bound to the lease it is searched on.
//!
//! [`RouteRequest`] freezes endpoints, arrival and the permission flags
//! production already exposes as booleans. Player facts / quest evidence,
//! avoid rectangles, the essence session and the node budget stay off this
//! surface until the acknowledged T5.W(route) interface change (L6).

use std::mem;

use api::snapshot::WorldTile;
use api::work::{InputRevision, Track, WorkCharge, WorkFamily, WorkFamilySpec, WorkPhase};

use crate::router::{RouteError, TargetError};

use super::{ArrivalSpec, BoundZoneExempt, ContractError, NavLease, PackId, RoutePlanLease};

/// Per-search opt-ins. Essence-session, avoid rects, player/quest evidence
/// and the expansion budget stay off this surface; T5.W(route) maps them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RoutePermissions {
    allow_teleports: bool,
    allow_wilderness: bool,
    allow_bank_fetch: bool,
    zones: Option<BoundZoneExempt>,
}

impl RoutePermissions {
    /// Safe defaults: no teleports, no wilderness, no bank fetch, no
    /// exemptions.
    pub const fn new() -> Self {
        Self {
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            zones: None,
        }
    }

    /// Allow any-tile teleports.
    pub const fn with_teleports(mut self, allow: bool) -> Self {
        self.allow_teleports = allow;
        self
    }

    /// Allow wilderness entry.
    pub const fn with_wilderness(mut self, allow: bool) -> Self {
        self.allow_wilderness = allow;
        self
    }

    /// Bank-fetch opt-in (does not itself insert a bank leg).
    pub const fn with_bank_fetch(mut self, allow: bool) -> Self {
        self.allow_bank_fetch = allow;
        self
    }

    /// Named pack-bound zone exemptions.
    pub const fn with_zones(mut self, zones: BoundZoneExempt) -> Self {
        self.zones = Some(zones);
        self
    }

    /// Teleport opt-in.
    pub const fn allow_teleports(self) -> bool {
        self.allow_teleports
    }

    /// Wilderness opt-in.
    pub const fn allow_wilderness(self) -> bool {
        self.allow_wilderness
    }

    /// Bank-fetch opt-in.
    pub const fn allow_bank_fetch(self) -> bool {
        self.allow_bank_fetch
    }

    /// Pack-bound zone exemptions, if any.
    pub const fn zones(self) -> Option<BoundZoneExempt> {
        self.zones
    }
}

/// A route request: endpoints, arrival rule, permissions and the family's
/// input revision. Session and job identity travel on
/// [`api::identity::WorkIdentity`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteRequest {
    from: WorldTile,
    to: WorldTile,
    arrival: ArrivalSpec,
    permissions: RoutePermissions,
    revision: InputRevision,
}

impl RouteRequest {
    /// A request from `from` to `to` at [`InputRevision::FIRST`].
    pub const fn new(from: WorldTile, to: WorldTile) -> Self {
        Self {
            from,
            to,
            arrival: ArrivalSpec::REACH,
            permissions: RoutePermissions::new(),
            revision: InputRevision::FIRST,
        }
    }

    /// Arrival rule.
    pub const fn with_arrival(mut self, arrival: ArrivalSpec) -> Self {
        self.arrival = arrival;
        self
    }

    /// Permissions.
    pub const fn with_permissions(mut self, permissions: RoutePermissions) -> Self {
        self.permissions = permissions;
        self
    }

    /// Input revision the producer assigned.
    pub const fn with_revision(mut self, revision: InputRevision) -> Self {
        self.revision = revision;
        self
    }

    /// Origin.
    pub const fn from(self) -> WorldTile {
        self.from
    }

    /// Destination.
    pub const fn to(self) -> WorldTile {
        self.to
    }

    /// Arrival.
    pub const fn arrival(self) -> ArrivalSpec {
        self.arrival
    }

    /// Permissions.
    pub const fn permissions(self) -> RoutePermissions {
        self.permissions
    }

    /// Input revision.
    pub const fn revision(self) -> InputRevision {
        self.revision
    }
}

impl RouteRequest {
    /// Bind this request to the lease it will be searched on. A request
    /// carrying another pack's exemption is refused with
    /// [`ContractError::PackMismatch`]; nothing else can carry a foreign
    /// pack index into a search. The bound request is
    /// [`RouteService`]'s input.
    pub fn bind(self, lease: &NavLease) -> Result<BoundRouteRequest, ContractError> {
        let pack = lease.identity().pack();
        if self
            .permissions
            .zones()
            .is_some_and(|zones| zones.pack() != pack)
        {
            return Err(ContractError::PackMismatch);
        }
        Ok(BoundRouteRequest {
            pack,
            request: self,
        })
    }
}

/// A [`RouteRequest`] bound to the pack it will be searched on. Built only
/// by [`RouteRequest::bind`], so every pack-bound value it carries belongs
/// to [`Self::pack`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundRouteRequest {
    pack: PackId,
    request: RouteRequest,
}

impl BoundRouteRequest {
    /// The lease this request is bound to.
    pub const fn pack(self) -> PackId {
        self.pack
    }

    /// The request.
    pub const fn request(self) -> RouteRequest {
        self.request
    }
}

/// Progress phases of a route search. Numerical allowances are G2b.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RoutePhase {
    /// Bounded expansions of the Dijkstra / shared search.
    Search,
    /// Publishing the identity-bound plan or NoPath.
    Commit,
}

impl RoutePhase {
    /// Stable index for [`api::work::WorkProgress::phase`].
    pub const fn index(self) -> u16 {
        match self {
            Self::Search => 0,
            Self::Commit => 1,
        }
    }
}

impl WorkPhase for RoutePhase {
    fn index(self) -> u16 {
        Self::index(self)
    }
}

/// Family result. [`Self::NoPath`] is a domain outcome, not an operational
/// failure (§26 §2.2). [`Clone`] shares the plan handle (`Arc`), not a bake
/// copy.
#[derive(Debug, Clone)]
pub enum RouteFamilyResult {
    /// An identity-bound plan.
    Path(RoutePlanLease),
    /// No walk/transport path reaches the destination (`RouteError::NoPath`).
    NoPath,
}

/// Family error. Budget exhaustion and an unsettled shared-search target
/// are not NoPath. Current producers: [`RouteError`] / [`TargetError`] in
/// `router.rs:155-178`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RouteFamilyError {
    /// The node-expansion budget was exhausted (`RouteError::BudgetExhausted`).
    #[error("route node-expansion budget exhausted")]
    BudgetExhausted,
    /// A shared search left this target unsettled (`TargetError::NotSettled`).
    #[error("shared route search left the target unsettled")]
    NotSettled,
}

/// Binding of [`RouteRequest`] / [`RouteFamilyResult`] / [`RouteFamilyError`]
/// to [`WorkFamily::Route`]. T5.W(route) drives the search; this type records
/// the family contract and maps current router errors.
pub struct RouteService;

impl RouteService {
    /// Producer of this family (the [`WorkFamily::Route`] table row).
    pub const fn producer() -> Track {
        Self::FAMILY.producer()
    }

    /// Consumers of this family (the [`WorkFamily::Route`] table row).
    pub const fn consumers() -> &'static [Track] {
        Self::FAMILY.consumers()
    }

    /// Map a current [`RouteError`] onto the frozen result/error split.
    pub const fn map_route_error(error: RouteError) -> Result<RouteFamilyResult, RouteFamilyError> {
        match error {
            RouteError::NoPath => Ok(RouteFamilyResult::NoPath),
            RouteError::BudgetExhausted => Err(RouteFamilyError::BudgetExhausted),
        }
    }

    /// Map a current [`TargetError`] onto the frozen result/error split.
    pub const fn map_target_error(
        error: TargetError,
    ) -> Result<RouteFamilyResult, RouteFamilyError> {
        match error {
            TargetError::NoPath => Ok(RouteFamilyResult::NoPath),
            TargetError::BudgetExhausted => Err(RouteFamilyError::BudgetExhausted),
            TargetError::NotSettled => Err(RouteFamilyError::NotSettled),
        }
    }
}

impl WorkFamilySpec for RouteService {
    const FAMILY: WorkFamily = WorkFamily::Route;
    type Input = BoundRouteRequest;
    type Output = RouteFamilyResult;
    type Error = RouteFamilyError;
    type Phase = RoutePhase;

    fn revision(input: &Self::Input) -> InputRevision {
        input.request().revision()
    }

    /// The bound request envelope. The plan lease is not reserved here: it
    /// is charged in its T5 owner ledger from publication (§26 §2.2,
    /// "caller-held result leases obey their own bounded owner ledger"), so
    /// result bytes are 0 rather than the `Arc` handle's size. Scratch and
    /// handles stay 0 until T1.2 publishes G2b numbers; input size alone is
    /// not the charge.
    fn charge(input: &Self::Input) -> WorkCharge {
        WorkCharge::new(mem::size_of_val(input), 0, 0, 0)
    }

    fn map_cancellation(reason: api::identity::Cancellation) -> api::identity::Cancellation {
        reason
    }
}
