//! Coordinates and arrival: [`api::snapshot::WorldTile`] plus Reach default
//! (with radius) and Area's plane/radius packed-bake standability rule.
//!
//! Packed-bake predicates on [`super::CollisionView`] answer over the bake,
//! exactly as the host answers packed route goals: Reach is
//! [`api::query::is_arrived`] over the bake's walkable word through
//! [`api::query::CollisionArrivalProbe`] (`host-play/src/script_nav.rs:1912-1917`
//! via `retain_arrival_candidates`, `api/src/query/reach.rs:1581-1587`), and
//! Area is [`crate::arrival::area_has_standable_goal`] /
//! [`crate::arrival::arrived_in_area`] over `WorldCollision::standable`
//! (`script/src/gatherer/select.rs:1085-1090`). They are not current
//! loaded-scene standability: live walks answer the same rules over the
//! scene (`script/src/native/walk.rs:49-62`, `api/src/snapshot/native.rs:110-126`),
//! and a Reach that names a loc needs the scene (`walk_loc_arrived`), so the
//! bake refuses it.

use api::query::{is_arrived, CollisionArrivalProbe};
use api::snapshot::WorldTile;

use super::{ArrivalKind, CollisionView, ContractError};

/// Native walk arrival rule plus a Chebyshev radius and optional loc
/// identity. Reach is the default. Radius is unsigned: a negative radius
/// cannot be represented.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArrivalSpec {
    kind: ArrivalKind,
    radius: u16,
    loc_id: Option<i32>,
}

impl ArrivalSpec {
    /// Traditional Reach at radius 0: standing on the destination.
    pub const REACH: Self = Self::reach(0);

    /// Reach with Chebyshev radius, the rule production walks carry on
    /// `WalkRequest.radius` and packed goals filter by
    /// (`api::query::is_arrived`): same level and within the radius, then
    /// standing on the destination, exact reach of a walkable destination,
    /// or adjacent reach of a solid destination through an open edge. The
    /// radius bounds the distance and the probe budget; it does not widen
    /// wall tolerance.
    pub const fn reach(radius: u16) -> Self {
        Self {
            kind: ArrivalKind::Reach,
            radius,
            loc_id: None,
        }
    }

    /// Area: any packed-bake standable tile in the Chebyshev radius on the
    /// same plane. Packed route goals are not proof of live standability.
    pub const fn area(radius: u16) -> Self {
        Self {
            kind: ArrivalKind::Area,
            radius,
            loc_id: None,
        }
    }

    /// Name a loc origin. Area + loc is refused (`native/walk.rs:57`). A
    /// Reach that names a loc is answered by the scene, never the bake
    /// ([`ContractError::LocArrivalNeedsScene`]).
    pub const fn with_loc(self, loc_id: i32) -> Result<Self, ContractError> {
        match self.kind {
            ArrivalKind::Area => Err(ContractError::AreaWithLoc),
            ArrivalKind::Reach => Ok(Self {
                loc_id: Some(loc_id),
                ..self
            }),
        }
    }

    /// Reach or Area.
    pub const fn kind(self) -> ArrivalKind {
        self.kind
    }

    /// Chebyshev radius. Meaningful for both Reach and Area.
    pub const fn radius(self) -> u16 {
        self.radius
    }

    /// Loc origin, if this Reach names one.
    pub const fn loc_id(self) -> Option<i32> {
        self.loc_id
    }
}

impl Default for ArrivalSpec {
    fn default() -> Self {
        Self::REACH
    }
}

pub(super) fn arrived(
    view: &CollisionView<'_>,
    here: WorldTile,
    dest: WorldTile,
    spec: ArrivalSpec,
) -> Result<bool, ContractError> {
    match spec.kind {
        ArrivalKind::Reach => {
            if spec.loc_id.is_some() {
                return Err(ContractError::LocArrivalNeedsScene);
            }
            // `script_nav.rs:1912-1917`: the bake's walkable word is the
            // collision surface; a tile outside the bake is unprobeable.
            let probe = CollisionArrivalProbe::new(|tile| view.collision_at(tile));
            Ok(is_arrived(here, dest, i32::from(spec.radius), || &probe))
        }
        ArrivalKind::Area => Ok(crate::arrival::arrived_in_area(
            here,
            dest,
            i32::from(spec.radius),
            |tile| view.standable(tile),
        )),
    }
}

pub(super) fn area_has_goal(view: &CollisionView<'_>, centre: WorldTile, radius: u16) -> bool {
    crate::arrival::area_has_standable_goal(centre, i32::from(radius), |tile| view.standable(tile))
}
