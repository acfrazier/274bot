//! Coordinates and arrival: [`api::snapshot::WorldTile`] plus Reach default
//! and Area's plane/radius/current-standability rule.

use api::snapshot::WorldTile;

use super::ArrivalKind;
use super::CollisionView;

/// Native walk arrival rule plus the Area radius. Reach is the default
/// and has no radius requirement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArrivalSpec {
    kind: ArrivalKind,
    radius: i32,
}

impl ArrivalSpec {
    /// Traditional Reach: the destination itself, or a solid target
    /// approached from an adjacent same-plane tile.
    pub const REACH: Self = Self {
        kind: ArrivalKind::Reach,
        radius: 0,
    };

    /// Area: any currently standable tile in the Chebyshev radius on the
    /// same plane. Packed route goals are not proof.
    pub const fn area(radius: i32) -> Self {
        Self {
            kind: ArrivalKind::Area,
            radius,
        }
    }

    /// Reach or Area.
    pub const fn kind(self) -> ArrivalKind {
        self.kind
    }

    /// Area radius; unused for Reach.
    pub const fn radius(self) -> i32 {
        self.radius
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
            let dest_walkable = view.walkable(dest);
            crate::arrival::arrived(as_tile(here), as_tile(dest), dest_walkable)
        }
        ArrivalKind::Area => {
            crate::arrival::arrived_in_area(here, dest, spec.radius, |tile| view.standable(tile))
        }
    }
}

pub(super) fn area_has_goal(view: &CollisionView<'_>, centre: WorldTile, radius: i32) -> bool {
    crate::arrival::area_has_standable_goal(centre, radius, |tile| view.standable(tile))
}

fn as_tile(tile: WorldTile) -> crate::tile::Tile {
    crate::tile::Tile {
        x: tile.x,
        z: tile.z,
        level: tile.level,
    }
}
