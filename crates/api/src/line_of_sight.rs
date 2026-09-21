//! Frozen line-of-sight ray and typed v1/v2 query order.
//!
//! Geometry is `hasLineOfSightLocal` plus the Reachability wrapper. This is
//! not the Reach flood.

use crate::snapshot::WorldTile;
use client::dash3d::CollisionFlag;
use std::sync::Arc;

/// Frozen scene size used for v1/v2 size caps. Independent of published width.
pub const SCENE_SIZE: i32 = 104;

const SIGHT_W: i32 = CollisionFlag::VIS_SCENERY | CollisionFlag::V_W;
const SIGHT_E: i32 = CollisionFlag::VIS_SCENERY | CollisionFlag::V_E;
const SIGHT_S: i32 = CollisionFlag::VIS_SCENERY | CollisionFlag::V_S;
const SIGHT_N: i32 = CollisionFlag::VIS_SCENERY | CollisionFlag::V_N;
const HALF_TILE: i32 = (1 << 16) / 2;

/// A footprint by its south-west tile, in scene-local coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Footprint {
    pub lx: i32,
    pub lz: i32,
    pub size: i32,
}

/// Posted one-plane collision identity the helper borrows.
#[derive(Clone, Debug)]
pub struct CollisionQuery {
    pub available: bool,
    pub base_x: i32,
    pub base_z: i32,
    pub level: i32,
    pub width: i32,
    pub height: i32,
    pub flags: Arc<[i32]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineOfSightError {
    InvalidArgs,
    MissingObservation,
}

impl LineOfSightError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidArgs => "invalid-args",
            Self::MissingObservation => "missing-observation",
        }
    }
}

/// The edge of a footprint the ray starts from or ends on, facing the other body.
pub fn coordinate(a: i32, b: i32, size: i32) -> i32 {
    if a >= b {
        a
    } else if a + size - 1 <= b {
        a + size - 1
    } else {
        b
    }
}

fn flagged(flags: &impl Fn(i32, i32) -> Option<i32>, lx: i32, lz: i32, mask: i32) -> bool {
    match flags(lx, lz) {
        None => true,
        Some(f) => (f & mask) != 0,
    }
}

/// Exact frozen DDA. `flags` returning `None` is opaque for stepped tiles.
pub fn has_line_of_sight_local(
    flags: &impl Fn(i32, i32) -> Option<i32>,
    src: Footprint,
    dest: Footprint,
) -> bool {
    let start_x = coordinate(src.lx, dest.lx, src.size);
    let start_z = coordinate(src.lz, dest.lz, src.size);
    let end_x = coordinate(dest.lx, src.lx, dest.size);
    let end_z = coordinate(dest.lz, src.lz, dest.size);
    if start_x == end_x && start_z == end_z {
        return true;
    }
    if flagged(flags, start_x, start_z, CollisionFlag::WALK_SCENERY) {
        return false;
    }
    let delta_x = end_x - start_x;
    let delta_z = end_z - start_z;
    let abs_x = delta_x.unsigned_abs() as i32;
    let abs_z = delta_z.unsigned_abs() as i32;
    let east = delta_x >= 0;
    let north = delta_z >= 0;
    let mut x_flags = if east { SIGHT_W } else { SIGHT_E };
    let mut z_flags = if north { SIGHT_S } else { SIGHT_N };
    if abs_x > abs_z {
        let step_x = if east { 1 } else { -1 };
        let mut scaled_z = start_z.wrapping_shl(16) + HALF_TILE + if north { 0 } else { -1 };
        let tangent = delta_z.wrapping_shl(16) / abs_x;
        let mut x = start_x;
        while x != end_x {
            x += step_x;
            let z = scaled_z >> 16;
            if x == end_x && z == end_z {
                x_flags &= !CollisionFlag::VIS_SCENERY;
            }
            if flagged(flags, x, z, x_flags) {
                return false;
            }
            scaled_z = scaled_z.wrapping_add(tangent);
            let next_z = scaled_z >> 16;
            if x == end_x && next_z == end_z {
                z_flags &= !CollisionFlag::VIS_SCENERY;
            }
            if next_z != z && flagged(flags, x, next_z, z_flags) {
                return false;
            }
        }
        return true;
    }
    let step_z = if north { 1 } else { -1 };
    let mut scaled_x = start_x.wrapping_shl(16) + HALF_TILE + if east { 0 } else { -1 };
    let tangent = delta_x.wrapping_shl(16) / abs_z;
    let mut z = start_z;
    while z != end_z {
        z += step_z;
        let x = scaled_x >> 16;
        if x == end_x && z == end_z {
            z_flags &= !CollisionFlag::VIS_SCENERY;
        }
        if flagged(flags, x, z, z_flags) {
            return false;
        }
        scaled_x = scaled_x.wrapping_add(tangent);
        let next_x = scaled_x >> 16;
        if next_x == end_x && z == end_z {
            x_flags &= !CollisionFlag::VIS_SCENERY;
        }
        if next_x != x && flagged(flags, next_x, z, x_flags) {
            return false;
        }
    }
    true
}

fn size_ok(size: i32) -> bool {
    (1..=SCENE_SIZE).contains(&size)
}

fn to_local(tile: WorldTile, q: &CollisionQuery) -> Option<(i32, i32)> {
    let lx = (tile.x as i64).checked_sub(q.base_x as i64)?;
    let lz = (tile.z as i64).checked_sub(q.base_z as i64)?;
    if lx < 0 || lz < 0 || lx >= q.width as i64 || lz >= q.height as i64 {
        return None;
    }
    Some((lx as i32, lz as i32))
}

fn flags_lookup(q: &CollisionQuery) -> impl Fn(i32, i32) -> Option<i32> + '_ {
    move |lx, lz| {
        if lx < 0 || lz < 0 || lx >= q.width || lz >= q.height {
            return None;
        }
        let idx = (lx as i64) * (q.height as i64) + (lz as i64);
        if idx < 0 || idx >= q.flags.len() as i64 {
            return None;
        }
        Some(q.flags[idx as usize])
    }
}

/// v2 order: invalid-args, different plane ok-false, missing family, origins, frozen ray.
pub fn line_of_sight_v2(
    collision: Option<&CollisionQuery>,
    from: WorldTile,
    to: WorldTile,
    size: Option<i32>,
) -> Result<bool, LineOfSightError> {
    let size = size.unwrap_or(1);
    if !size_ok(size) {
        return Err(LineOfSightError::InvalidArgs);
    }
    if from.level != to.level {
        return Ok(false);
    }
    let Some(q) = collision.filter(|c| c.available) else {
        return Err(LineOfSightError::MissingObservation);
    };
    if q.level != from.level {
        return Err(LineOfSightError::MissingObservation);
    }
    let Some((from_lx, from_lz)) = to_local(from, q) else {
        return Ok(false);
    };
    let Some((to_lx, to_lz)) = to_local(to, q) else {
        return Ok(false);
    };
    let at = flags_lookup(q);
    Ok(has_line_of_sight_local(
        &at,
        Footprint {
            lx: from_lx,
            lz: from_lz,
            size: 1,
        },
        Footprint {
            lx: to_lx,
            lz: to_lz,
            size,
        },
    ))
}

/// v1: invalid / missing / wrong plane / out of scene → false. No throw.
pub fn line_of_sight_v1(
    collision: Option<&CollisionQuery>,
    from: WorldTile,
    to: WorldTile,
    size: Option<i32>,
) -> bool {
    match line_of_sight_v2(collision, from, to, size) {
        Ok(v) => v,
        Err(_) => false,
    }
}

/// Public raw index `lx * height + lz` on the posted identity.
pub fn flag_at(q: &CollisionQuery, index: i32) -> Option<i32> {
    if !q.available {
        return None;
    }
    if index < 0 {
        return None;
    }
    q.flags.get(index as usize).copied()
}
