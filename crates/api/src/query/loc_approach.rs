//! Where a footprint loc can be operated from (the m8aq
//! `LocApproach.ts`): a 4-bit force-approach mask rotated by the
//! loc's angle, checked against the scene's directional wall flags.

use crate::snapshot::{GameSnapshot, LocView, SceneView, WorldTile};
use client::dash3d::CollisionFlag;

/// The loc shapes with a real footprint (the m8aq
/// `FOOTPRINT_SHAPES`); other shapes read as "don't know".
const FOOTPRINT_SHAPES: [i32; 3] = [10, 11, 22];

const FORCE_NORTH: i32 = 0x1;
const FORCE_EAST: i32 = 0x2;
const FORCE_SOUTH: i32 = 0x4;
const FORCE_WEST: i32 = 0x8;

/// A placed footprint's rotated dimensions and blocked approach sides.
/// Shared by live loc readiness and packed transport admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocApproach {
    pub width: u8,
    pub length: u8,
    pub blocked_sides: u8,
}

impl LocApproach {
    /// Construct the packed approach geometry from a loc definition's raw,
    /// unrotated dimensions and force-approach mask.
    ///
    /// Returns `None` when a dimension cannot be represented by the packed
    /// footprint fields.
    pub fn from_loc_def(width: i32, length: i32, angle: i32, force_approach: i32) -> Option<Self> {
        let (width, length) = if angle & 1 == 0 {
            (width, length)
        } else {
            (length, width)
        };
        Some(Self {
            width: u8::try_from(width).ok()?,
            length: u8::try_from(length).ok()?,
            blocked_sides: rotate_force_approach(force_approach, angle) as u8,
        })
    }

    /// Test one stand against the footprint, not merely its south-west anchor.
    /// Callers check scene bounds and standability before admission.
    pub fn can_operate(self, origin: WorldTile, from: WorldTile, flags: i32) -> bool {
        if from.level != origin.level || self.width == 0 || self.length == 0 {
            return false;
        }
        let max_x = origin.x + i32::from(self.width) - 1;
        let max_z = origin.z + i32::from(self.length) - 1;
        let in_x = (origin.x..=max_x).contains(&from.x);
        let in_z = (origin.z..=max_z).contains(&from.z);
        let mask = i32::from(self.blocked_sides);
        (in_x && in_z)
            || (from.x == origin.x - 1
                && in_z
                && flags & CollisionFlag::W_E == 0
                && mask & FORCE_WEST == 0)
            || (from.x == max_x + 1
                && in_z
                && flags & CollisionFlag::W_W == 0
                && mask & FORCE_EAST == 0)
            || (from.z == origin.z - 1
                && in_x
                && flags & CollisionFlag::W_N == 0
                && mask & FORCE_SOUTH == 0)
            || (from.z == max_z + 1
                && in_x
                && flags & CollisionFlag::W_S == 0
                && mask & FORCE_NORTH == 0)
    }
}

/// Rotate the 4-bit force-approach mask by the loc's angle (the m8aq
/// `rotateForceApproach`).
fn rotate_force_approach(force_approach: i32, angle: i32) -> i32 {
    if angle == 0 {
        return force_approach;
    }
    ((force_approach << angle) & 0xf) | (force_approach >> (4 - angle))
}

/// The flags of a local tile; out of bounds (or a missing flag) reads
/// as fully blocked.
fn collision_at(scene: &SceneView, lx: i32, lz: i32) -> i32 {
    if lx < 0 || lz < 0 || lx >= scene.width || lz >= scene.height {
        return CollisionFlag::SQ_BLOCKED;
    }
    scene
        .collision_flags
        .get((lx * scene.height + lz) as usize)
        .copied()
        .unwrap_or(CollisionFlag::SQ_BLOCKED)
}

/// Whether `src` can interact with the footprint `dst..dst+size`:
/// standing inside it, or adjacent on a side whose wall flag and
/// force-approach bit are clear (the m8aq `testLoc`).
#[allow(clippy::too_many_arguments)]
fn test_loc(
    src_x: i32,
    src_z: i32,
    dst_x: i32,
    dst_z: i32,
    size_x: i32,
    size_z: i32,
    force_approach: i32,
    scene: &SceneView,
) -> bool {
    let (Ok(width), Ok(length)) = (u8::try_from(size_x), u8::try_from(size_z)) else {
        return false;
    };
    LocApproach {
        width,
        length,
        blocked_sides: force_approach as u8,
    }
    .can_operate(
        WorldTile {
            x: dst_x,
            z: dst_z,
            level: scene.level,
        },
        WorldTile {
            x: src_x,
            z: src_z,
            level: scene.level,
        },
        collision_at(scene, src_x, src_z),
    )
}

/// Whether `from` can operate `loc` in `scene`; `None` when the
/// loc's shape has no approach model (or the scene/level don't line
/// up), `Some(false)` when the tile is out of bounds or blocked.
pub fn can_operate_from(loc: &LocView, scene: &SceneView, from: WorldTile) -> Option<bool> {
    if !FOOTPRINT_SHAPES.contains(&loc.shape) {
        return None;
    }
    if !scene.available || from.level != scene.level || loc.tile.level != scene.level {
        return None;
    }
    let src_x = from.x - scene.base_x;
    let src_z = from.z - scene.base_z;
    if src_x < 0 || src_z < 0 || src_x >= scene.width || src_z >= scene.height {
        return Some(false);
    }
    let dst_x = loc.tile.x - scene.base_x;
    let dst_z = loc.tile.z - scene.base_z;
    let force_approach = rotate_force_approach(loc.force_approach, loc.angle);
    Some(test_loc(
        src_x,
        src_z,
        dst_x,
        dst_z,
        loc.footprint_width,
        loc.footprint_length,
        force_approach,
        scene,
    ))
}

/// Chebyshev distance from a tile to the loc's full rotated rectangle.
/// A loc-backed radius is measured from this footprint, not its south-west
/// anchor. Plain tile destinations retain their anchor-based radius.
pub fn distance_from(loc: &LocView, from: WorldTile) -> Option<u32> {
    if !FOOTPRINT_SHAPES.contains(&loc.shape)
        || from.level != loc.tile.level
        || loc.footprint_width <= 0
        || loc.footprint_length <= 0
    {
        return None;
    }
    let nearest_x = from
        .x
        .clamp(loc.tile.x, loc.tile.x + loc.footprint_width - 1);
    let nearest_z = from
        .z
        .clamp(loc.tile.z, loc.tile.z + loc.footprint_length - 1);
    Some(from.x.abs_diff(nearest_x).max(from.z.abs_diff(nearest_z)))
}

/// Live footprint arrival for a radius walk. An off-scene collision estimate
/// is not an interaction proof: once the loc is visible, only its real rotated
/// footprint, force-approach sides and live wall flags may settle the walk.
/// The Chebyshev distance to that full rectangle must be within `radius`;
/// perimeter stands are not admitted independently of the requested margin.
/// `None` preserves ordinary walk arrival for an unmodeled destination.
pub fn arrived_at(
    snapshot: &GameSnapshot,
    from: WorldTile,
    to: WorldTile,
    radius: i32,
) -> Option<bool> {
    let scene = snapshot.scene();
    let query = super::SceneQuery::new(scene, None);
    if !query.contains(to) {
        return None;
    }
    let mut modeled = false;
    for loc in snapshot.locs().iter().filter(|loc| loc.tile == to) {
        let Some(can_operate) = can_operate_from(loc, scene, from) else {
            continue;
        };
        modeled = true;
        let in_radius = u32::try_from(radius).is_ok_and(|radius| {
            distance_from(loc, from).is_some_and(|distance| distance <= radius)
        });
        if in_radius && query.walkable(from) && can_operate {
            return Some(true);
        }
    }
    modeled.then_some(false)
}

/// Every walkable tile from which `loc` can be operated; `None` when
/// the loc's shape has no approach model.
pub fn operable_tiles(loc: &LocView, scene: &SceneView) -> Option<Vec<WorldTile>> {
    if !FOOTPRINT_SHAPES.contains(&loc.shape) {
        return None;
    }
    if !scene.available || loc.tile.level != scene.level {
        return None;
    }
    let dst_x = loc.tile.x - scene.base_x;
    let dst_z = loc.tile.z - scene.base_z;
    let force_approach = rotate_force_approach(loc.force_approach, loc.angle);
    let size_x = loc.footprint_width;
    let size_z = loc.footprint_length;
    let mut tiles = Vec::new();
    for lx in (dst_x - 1).max(0)..=(dst_x + size_x).min(scene.width - 1) {
        for lz in (dst_z - 1).max(0)..=(dst_z + size_z).min(scene.height - 1) {
            if collision_at(scene, lx, lz) & CollisionFlag::SQ_BLOCKED != 0 {
                continue;
            }
            if test_loc(lx, lz, dst_x, dst_z, size_x, size_z, force_approach, scene) {
                tiles.push(WorldTile {
                    x: scene.base_x + lx,
                    z: scene.base_z + lz,
                    level: scene.level,
                });
            }
        }
    }
    Some(tiles)
}

/// Compact operate-from-here + exact stand dest for one footprint loc.
/// `None` when the loc has no approach model. Dest is the closest
/// flood-reachable operable tile (cheb, then x, then z); already-ready
/// keeps `from` and does not walk off it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoothApproach {
    pub can_operate: bool,
    pub dest: Option<WorldTile>,
}

/// One `test_loc` producer: readiness from [`can_operate_from`], dest
/// from [`operable_tiles`] filtered by an existing [`super::ReachFlood`]
/// exact stand. No second BFS and no world copy.
pub fn booth_approach(
    loc: &LocView,
    scene: &SceneView,
    from: WorldTile,
    flood: &super::ReachFlood,
) -> Option<BoothApproach> {
    let can_operate = can_operate_from(loc, scene, from)?;
    if can_operate {
        return Some(BoothApproach {
            can_operate: true,
            dest: Some(from),
        });
    }
    let dest = operable_tiles(loc, scene)?
        .into_iter()
        .filter(|tile| flood.at(tile).0)
        .min_by_key(|tile| {
            let cheb = (from.x - tile.x).abs().max((from.z - tile.z).abs());
            (cheb, tile.x, tile.z)
        });
    Some(BoothApproach {
        can_operate: false,
        dest,
    })
}
