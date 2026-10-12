//! Borrowed collision view: plane-aware packed-bake standable, walkable,
//! step validity and arrival/goal queries. Does not expose `WorldCollision`
//! ownership, walk/blocked/flags vectors, mutable storage or a world copy.
//!
//! These predicates read the packed bake. They are not current loaded-scene
//! standability (`native/walk.rs` / `script_walk.rs`). Gatherer uses the bake
//! (`gatherer/select.rs:1085-1088`).

use std::fmt;

use api::snapshot::WorldTile;

use crate::collision::WorldCollision;

use super::arrival::{area_has_goal, arrived};
use super::{ArrivalSpec, PackId};

/// Concrete borrowed collision view. Copying it copies the borrow, not the
/// bake. Gatherer's fixed-stack local search stays in Gatherer and reads
/// these queries; this view does not replace it with a router allocation.
#[derive(Clone, Copy)]
pub struct CollisionView<'a> {
    pack: PackId,
    collision: &'a WorldCollision,
}

impl<'a> CollisionView<'a> {
    pub(super) fn new(pack: PackId, collision: &'a WorldCollision) -> Self {
        Self { pack, collision }
    }

    /// Originating pack lease.
    pub const fn pack(self) -> PackId {
        self.pack
    }

    /// Plane-aware grid membership (levels 0..=3 inside the bake bbox).
    pub fn contains_tile(self, tile: WorldTile) -> bool {
        if !(0..4).contains(&tile.level) {
            return false;
        }
        let lx = tile.x - self.collision.origin.x;
        let lz = tile.z - self.collision.origin.z;
        lx >= 0
            && lz >= 0
            && (lx as usize) < self.collision.width
            && (lz as usize) < self.collision.height
    }

    /// Packed-bake footprint/ground standability. Face flags do not
    /// disqualify standing (`collision.rs:219-237`). Not live-scene
    /// standability.
    pub fn standable(self, tile: WorldTile) -> bool {
        self.collision.standable(tile)
    }

    /// Packed-bake blanket walkability, stricter than standable: any face
    /// bit or `SQ_BLOCKED` rejects the tile.
    pub fn walkable(self, tile: WorldTile) -> bool {
        self.collision.walkable(tile)
    }

    /// One-tile step validity: the client's directional `tryMove` test
    /// (`router.rs:3335-3387`), including diagonal orthogonal-face rules.
    /// Different planes or a non-adjacent pair are not a step. The plane
    /// check is this view's: `router::step_ok` copies `from.level` onto the
    /// neighbour and would otherwise ignore `to.level`.
    pub fn step_ok(self, from: WorldTile, to: WorldTile) -> bool {
        if from.level != to.level {
            return false;
        }
        let d = (to.x - from.x, to.z - from.z);
        if d == (0, 0) {
            return false;
        }
        crate::router::step_ok(self.collision, from, d)
    }

    /// Packed-bake Reach or Area arrival. Reach uses bake walkable (solid
    /// destinations are approached); Area uses bake standable. See
    /// [`ArrivalSpec`].
    pub fn arrived(self, here: WorldTile, dest: WorldTile, spec: ArrivalSpec) -> bool {
        arrived(&self, here, dest, spec)
    }

    /// Packed-bake Area goals: any bake-standable tile in the Chebyshev
    /// neighbourhood. Empty means a Walk has no Area goals.
    pub fn area_has_goal(self, centre: WorldTile, radius: u16) -> bool {
        area_has_goal(&self, centre, radius)
    }
}

impl fmt::Debug for CollisionView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CollisionView")
            .field("pack", &self.pack)
            .finish_non_exhaustive()
    }
}
