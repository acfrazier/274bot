//! Borrowed collision view: plane-aware packed-bake standable, walkable,
//! step validity and arrival/goal queries. Does not expose `WorldCollision`
//! ownership, walk/blocked/flags vectors, mutable storage or a world copy.
//!
//! These predicates read the packed bake. They are not current loaded-scene
//! standability (`native/walk.rs` / `script_walk.rs`). Gatherer uses the bake
//! (`gatherer/select.rs:1085-1090`); the host filters packed Reach goals over
//! it (`script_nav.rs:1912-1917`).

use std::fmt;

use api::snapshot::WorldTile;

use crate::collision::WorldCollision;

use super::arrival::{area_has_goal, arrived};
use super::{ArrivalSpec, ContractError, PackId};

/// Concrete borrowed collision view. Copying it copies the borrow, not the
/// bake. Gatherer's fixed-stack local search stays in Gatherer and reads
/// these queries; this view does not replace it with a router allocation.
#[derive(Clone, Copy)]
pub struct CollisionView<'a> {
    pack: PackId,
    collision: &'a WorldCollision,
}

impl<'a> CollisionView<'a> {
    pub(crate) fn new(pack: PackId, collision: &'a WorldCollision) -> Self {
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

    /// The bake's collision surface for [`api::query::CollisionArrivalProbe`]:
    /// the walkable word of an in-bake tile, `None` outside the bake or
    /// off-plane. Byte-for-byte the closure the host hands to
    /// `retain_arrival_candidates` (`host-play/src/script_nav.rs:1913-1916`).
    pub(super) fn collision_at(self, tile: WorldTile) -> Option<i32> {
        let collision = self.collision;
        let x = usize::try_from(tile.x.checked_sub(collision.origin.x)?).ok()?;
        let z = usize::try_from(tile.z.checked_sub(collision.origin.z)?).ok()?;
        ((0..=3).contains(&tile.level) && x < collision.width && z < collision.height)
            .then(|| collision.walkable_word(tile.x, tile.z, tile.level) as i32)
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
    /// in `movement`, including diagonal orthogonal-face rules.
    /// Different planes or a non-adjacent pair are not a step. The plane
    /// check is this view's: `movement::step_ok` copies `from.level` onto the
    /// neighbour and would otherwise ignore `to.level`.
    pub fn step_ok(self, from: WorldTile, to: WorldTile) -> bool {
        if from.level != to.level {
            return false;
        }
        let d = (to.x - from.x, to.z - from.z);
        if d == (0, 0) {
            return false;
        }
        crate::movement::step_ok(self.collision, from, d)
    }

    /// Packed-bake Reach or Area arrival. Reach is production
    /// [`api::query::is_arrived`] over the bake's walkable word (solid
    /// destinations are approached through an open edge); Area uses bake
    /// standable. A Reach that names a loc is refused: only the scene can
    /// answer it. See [`ArrivalSpec`].
    pub fn arrived(
        self,
        here: WorldTile,
        dest: WorldTile,
        spec: ArrivalSpec,
    ) -> Result<bool, ContractError> {
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
