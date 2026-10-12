//! Service envelope: permissions/admission and typed route terminals.
//! Concrete views borrow the originating [`super::NavLease`].

use api::snapshot::WorldTile;

use super::{ArrivalSpec, RoutePlanLease, ZoneExempt};

/// Per-search opt-ins. Essence-session and other backend-only flags stay
/// off this surface; T5.W maps them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RoutePermissions {
    allow_teleports: bool,
    allow_wilderness: bool,
    allow_bank_fetch: bool,
    zones: ZoneExempt,
}

impl RoutePermissions {
    /// Safe defaults: no teleports, no wilderness, no bank fetch, no
    /// exemptions.
    pub const fn new() -> Self {
        Self {
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            zones: ZoneExempt::NONE,
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

    /// Named zone exemptions.
    pub const fn with_zones(mut self, zones: ZoneExempt) -> Self {
        self.zones = zones;
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

    /// Zone exemptions.
    pub const fn zones(self) -> ZoneExempt {
        self.zones
    }
}

/// A route request: endpoints, arrival rule and permissions. Session and
/// request identity travel on [`api::identity::WorkIdentity`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteRequest {
    from: WorldTile,
    to: WorldTile,
    arrival: ArrivalSpec,
    permissions: RoutePermissions,
}

impl RouteRequest {
    /// A request from `from` to `to`.
    pub const fn new(from: WorldTile, to: WorldTile) -> Self {
        Self {
            from,
            to,
            arrival: ArrivalSpec::REACH,
            permissions: RoutePermissions::new(),
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
}

/// Family result. [`Self::NoPath`] is a domain outcome, not an operational
/// failure (§26 §2.2).
#[derive(Debug, Clone)]
pub enum RouteFamilyResult {
    /// An identity-bound plan.
    Path(RoutePlanLease),
    /// No walk/transport path reaches the destination.
    NoPath,
}

/// Family error. Budget exhaustion is not NoPath.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteFamilyError {
    /// The node-expansion budget was exhausted.
    BudgetExhausted,
}
