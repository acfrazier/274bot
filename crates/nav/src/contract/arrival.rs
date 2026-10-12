//! Coordinates and arrival: [`api::snapshot::WorldTile`] plus Reach default
//! (with radius) and Area's plane/radius packed-bake standability rule.
//!
//! Packed-bake predicates on [`super::CollisionView`] are Gatherer's use of
//! the bake (`gatherer/select.rs`). They are not current loaded-scene
//! standability. Live arrival is `native/walk.rs`: Area uses the scene's
//! walkability probe; `(Area, Some(loc))` is refused.

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

    /// Reach with Chebyshev radius. Production walks carry this on
    /// `WalkRequest.radius` (`native.rs`). Radius 0 is on-destination only;
    /// radius 1 admits the adjacent-solid approach `nav::arrival::arrived`
    /// uses for a packed bake.
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

    /// Name a loc origin. Area + loc is refused (`native/walk.rs:57`).
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
) -> bool {
    match spec.kind {
        ArrivalKind::Reach => {
            if here.level != dest.level {
                return false;
            }
            let dist = here.x.abs_diff(dest.x).max(here.z.abs_diff(dest.z));
            if dist > u32::from(spec.radius) {
                return false;
            }
            if dist == 0 {
                return true;
            }
            let dest_walkable = view.walkable(dest);
            crate::arrival::arrived(as_tile(here), as_tile(dest), dest_walkable)
        }
        ArrivalKind::Area => {
            if spec.loc_id.is_some() {
                return false;
            }
            crate::arrival::arrived_in_area(here, dest, i32::from(spec.radius), |tile| {
                view.standable(tile)
            })
        }
    }
}

pub(super) fn area_has_goal(view: &CollisionView<'_>, centre: WorldTile, radius: u16) -> bool {
    crate::arrival::area_has_standable_goal(centre, i32::from(radius), |tile| view.standable(tile))
}

fn as_tile(tile: WorldTile) -> crate::tile::Tile {
    crate::tile::Tile {
        x: tile.x,
        z: tile.z,
        level: tile.level,
    }
}
