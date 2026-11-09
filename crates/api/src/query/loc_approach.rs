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

/// Whether an identified loc is absent from an observed, ready scene.
/// Off-scene and rebuilding targets are unknown, not gone.
pub fn target_gone(snapshot: &GameSnapshot, to: WorldTile, loc_id: i32) -> bool {
    snapshot.ingame()
        && snapshot.scene_state() == 2
        && super::SceneQuery::new(snapshot.scene(), None).contains(to)
        && !snapshot
            .locs()
            .iter()
            .any(|loc| loc.tile == to && loc.id == loc_id)
}

/// Live footprint arrival for an explicitly identified loc radius walk.
/// Off-scene or unknown footprints return `None`, not interaction proof.
/// The full rotated rectangle, force-approach sides and live wall flags
/// determine arrival; unrelated locs on the destination tile are ignored.
pub fn arrived_at(
    snapshot: &GameSnapshot,
    from: WorldTile,
    to: WorldTile,
    radius: i32,
    loc_id: i32,
) -> Option<bool> {
    let scene = snapshot.scene();
    let query = super::SceneQuery::new(scene, None);
    if !query.contains(to) {
        return None;
    }
    let mut modeled = false;
    for loc in snapshot
        .locs()
        .iter()
        .filter(|loc| loc.tile == to && loc.id == loc_id)
    {
        if distance_from(loc, to).is_none() {
            continue;
        }
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
    Some(operable_tiles_iter(loc, scene)?.collect())
}

fn operable_tiles_iter<'a>(
    loc: &'a LocView,
    scene: &'a SceneView,
) -> Option<impl Iterator<Item = WorldTile> + 'a> {
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
    Some(
        ((dst_x - 1).max(0)..=(dst_x + size_x).min(scene.width - 1)).flat_map(move |lx| {
            ((dst_z - 1).max(0)..=(dst_z + size_z).min(scene.height - 1)).filter_map(move |lz| {
                (collision_at(scene, lx, lz) & CollisionFlag::SQ_BLOCKED == 0
                    && test_loc(lx, lz, dst_x, dst_z, size_x, size_z, force_approach, scene))
                .then_some(WorldTile {
                    x: scene.base_x + lx,
                    z: scene.base_z + lz,
                    level: scene.level,
                })
            })
        }),
    )
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
    booth_approach_by(loc, scene, from, |tile| flood.at(&tile).0)
}

pub(crate) fn booth_approach_by(
    loc: &LocView,
    scene: &SceneView,
    from: WorldTile,
    exact_reachable: impl Fn(WorldTile) -> bool,
) -> Option<BoothApproach> {
    let can_operate = can_operate_from(loc, scene, from)?;
    if can_operate {
        return Some(BoothApproach {
            can_operate: true,
            dest: Some(from),
        });
    }
    let dest = operable_tiles_iter(loc, scene)?
        .filter(|tile| exact_reachable(*tile))
        .min_by_key(|tile| {
            let cheb = (from.x - tile.x).abs().max((from.z - tile.z).abs());
            (cheb, tile.x, tile.z)
        });
    Some(BoothApproach {
        can_operate: false,
        dest,
    })
}

/// Engine `ReachStrategy.exitStrategy` family for a packed loc shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocReachStrategy {
    /// Shapes 0..=3 and 9: wall.
    Wall,
    /// Shapes 4..=8: wall decoration.
    WallDecor,
    /// Shapes 10, 11, and 22: rotated footprint.
    Footprint,
    /// Roofs and other shapes the engine does not interact through.
    Unsupported,
}

/// Classify a loc shape into the engine reach family.
pub fn loc_reach_strategy(shape: u8) -> LocReachStrategy {
    match shape {
        0..=3 | 9 => LocReachStrategy::Wall,
        4..=8 => LocReachStrategy::WallDecor,
        10 | 11 | 22 => LocReachStrategy::Footprint,
        _ => LocReachStrategy::Unsupported,
    }
}

/// Inputs for [`loc_can_operate`].
pub struct LocOperateQuery {
    pub origin: WorldTile,
    pub from: WorldTile,
    pub shape: u8,
    pub angle: u8,
    pub width: i32,
    pub length: i32,
    pub force_approach: i32,
    /// Collision word of the stand tile (`from`): the client `CollisionFlag`
    /// layout, so `PL_WALK_*` and `W_*` masks read it directly.
    pub flags: i32,
}

/// Whether `query.from` can operate the placed loc using the engine size-1
/// strategy. `None` is unsupported geometry.
pub fn loc_can_operate(query: LocOperateQuery) -> Option<bool> {
    if query.from.level != query.origin.level {
        return Some(false);
    }
    match loc_reach_strategy(query.shape) {
        LocReachStrategy::Unsupported => None,
        LocReachStrategy::Footprint => {
            let approach = LocApproach::from_loc_def(
                query.width,
                query.length,
                i32::from(query.angle),
                query.force_approach,
            )?;
            Some(approach.can_operate(query.origin, query.from, query.flags))
        }
        LocReachStrategy::Wall => Some(wall_can_operate(
            query.origin,
            query.from,
            query.shape,
            query.angle,
            query.flags,
        )),
        LocReachStrategy::WallDecor => Some(wall_decor_can_operate(
            query.origin,
            query.from,
            query.shape,
            query.angle,
            query.flags,
        )),
    }
}

/// One neighbour of the placed origin: `(dx, dz, mask)` of the stand tile
/// relative to the origin. The stand is legal when `flags & mask` is clear;
/// mask `0` means the neighbour needs no flag test.
type ReachSide = (i32, i32, i32);

/// Engine `ReachStrategy.reachWall1` (`ReachStrategy.ts:98-173`): the
/// stand-tile neighbours of a size-1 wall. The engine masks `BLOCK_NORTH`,
/// `BLOCK_EAST`, `BLOCK_SOUTH` and `BLOCK_WEST` are the client `PL_WALK_S`,
/// `PL_WALK_W`, `PL_WALK_N` and `PL_WALK_E` (same wall bit, same `LOC`
/// bit). Shapes 1 and 3 (diagonal and square corners) have no branch in the
/// engine: only the origin tile reaches them.
fn wall_sides(shape: u8, angle: u8) -> &'static [ReachSide] {
    const S: i32 = CollisionFlag::PL_WALK_S;
    const W: i32 = CollisionFlag::PL_WALK_W;
    const N: i32 = CollisionFlag::PL_WALK_N;
    const E: i32 = CollisionFlag::PL_WALK_E;
    match (shape, angle) {
        (0, 0) => &[(-1, 0, 0), (0, 1, S), (0, -1, N)],
        (0, 1) => &[(0, 1, 0), (-1, 0, E), (1, 0, W)],
        (0, 2) => &[(1, 0, 0), (0, 1, S), (0, -1, N)],
        (0, 3) => &[(0, -1, 0), (-1, 0, E), (1, 0, W)],
        (2, 0) => &[(-1, 0, 0), (0, 1, 0), (1, 0, W), (0, -1, N)],
        (2, 1) => &[(-1, 0, E), (0, 1, 0), (1, 0, 0), (0, -1, N)],
        (2, 2) => &[(-1, 0, E), (0, 1, S), (1, 0, 0), (0, -1, 0)],
        (2, 3) => &[(-1, 0, 0), (0, 1, S), (1, 0, W), (0, -1, 0)],
        (9, _) => &[
            (0, 1, CollisionFlag::W_S),
            (0, -1, CollisionFlag::W_N),
            (-1, 0, CollisionFlag::W_E),
            (1, 0, CollisionFlag::W_W),
        ],
        _ => &[],
    }
}

/// Engine `ReachStrategy.reachWallDecor1` (`ReachStrategy.ts:254-280`). Shapes
/// 4 and 5 (straight decorations) have no branch: only the origin tile
/// reaches them. Shapes 6 and 7 test two tiles chosen by the rotation, shape
/// 7 after `alteredRotation` (`angle + 2`). Shape 8 tests all four sides.
fn wall_decor_sides(shape: u8, angle: u8) -> &'static [ReachSide] {
    const NORTH_SIDE: i32 = CollisionFlag::W_N;
    const EAST_SIDE: i32 = CollisionFlag::W_E;
    const SOUTH_SIDE: i32 = CollisionFlag::W_S;
    const WEST_SIDE: i32 = CollisionFlag::W_W;
    let angle = if shape == 7 { (angle + 2) & 3 } else { angle };
    match (shape, angle) {
        (6 | 7, 0) => &[(1, 0, WEST_SIDE), (0, -1, NORTH_SIDE)],
        (6 | 7, 1) => &[(-1, 0, EAST_SIDE), (0, -1, NORTH_SIDE)],
        (6 | 7, 2) => &[(-1, 0, EAST_SIDE), (0, 1, SOUTH_SIDE)],
        (6 | 7, 3) => &[(1, 0, WEST_SIDE), (0, 1, SOUTH_SIDE)],
        (8, _) => &[
            (0, 1, SOUTH_SIDE),
            (0, -1, NORTH_SIDE),
            (-1, 0, EAST_SIDE),
            (1, 0, WEST_SIDE),
        ],
        _ => &[],
    }
}

fn sides_allow(sides: &[ReachSide], origin: WorldTile, from: WorldTile, flags: i32) -> bool {
    if from == origin {
        return true;
    }
    let dx = from.x - origin.x;
    let dz = from.z - origin.z;
    sides
        .iter()
        .any(|&(sx, sz, mask)| sx == dx && sz == dz && flags & mask == CollisionFlag::_OPEN)
}

fn wall_can_operate(origin: WorldTile, from: WorldTile, shape: u8, angle: u8, flags: i32) -> bool {
    sides_allow(wall_sides(shape, angle), origin, from, flags)
}

fn wall_decor_can_operate(
    origin: WorldTile,
    from: WorldTile,
    shape: u8,
    angle: u8,
    flags: i32,
) -> bool {
    sides_allow(wall_decor_sides(shape, angle), origin, from, flags)
}

/// Candidate tiles to test for a size-1 interaction, including the origin.
pub fn loc_candidate_stands(
    origin: WorldTile,
    shape: u8,
    angle: u8,
    width: i32,
    length: i32,
) -> Option<Vec<WorldTile>> {
    match loc_reach_strategy(shape) {
        LocReachStrategy::Unsupported => None,
        LocReachStrategy::Footprint => {
            let approach = LocApproach::from_loc_def(width, length, i32::from(angle), 0)?;
            let max_x = origin.x + i32::from(approach.width);
            let max_z = origin.z + i32::from(approach.length);
            let mut tiles = Vec::new();
            for x in (origin.x - 1)..=max_x {
                for z in (origin.z - 1)..=max_z {
                    tiles.push(WorldTile {
                        x,
                        z,
                        level: origin.level,
                    });
                }
            }
            Some(tiles)
        }
        LocReachStrategy::Wall | LocReachStrategy::WallDecor => Some(vec![
            origin,
            WorldTile {
                x: origin.x - 1,
                z: origin.z,
                level: origin.level,
            },
            WorldTile {
                x: origin.x + 1,
                z: origin.z,
                level: origin.level,
            },
            WorldTile {
                x: origin.x,
                z: origin.z - 1,
                level: origin.level,
            },
            WorldTile {
                x: origin.x,
                z: origin.z + 1,
                level: origin.level,
            },
        ]),
    }
}
