use super::*;

/// The player's world tile from the snapshot: the canonical route-based
/// tile (`base + route_x[0]`, the server-confirmed position), the same
/// source the settle `arrived` arm and the runner's `arrived` proof read.
/// `(0, 0, 0)` before the first `PLAYER_INFO` (the m8aq `here()`
/// fallback).
pub(super) fn here(snapshot: &GameSnapshot) -> WorldTile {
    snapshot
        .tile()
        .map(|(x, z, level)| WorldTile { x, z, level })
        .unwrap_or(WorldTile {
            x: 0,
            z: 0,
            level: 0,
        })
}

/// Chebyshev distance between world tiles (level ignored, like `nav::tile`).
pub(super) fn cheb(a: WorldTile, b: WorldTile) -> i32 {
    (a.x - b.x).abs().max((a.z - b.z).abs())
}

pub(super) fn tile_in_scene(scene: &api::snapshot::SceneView, t: WorldTile) -> bool {
    if scene.width <= 0 || scene.height <= 0 {
        return true;
    }
    let lx = t.x - scene.base_x;
    let lz = t.z - scene.base_z;
    lx >= 0 && lz >= 0 && lx < scene.width && lz < scene.height
}

/// The eight tiles within chebyshev 1 of a tile, swept in a fixed order so
/// the nearest-approach choice is deterministic.
pub(super) const APPROACH_RING: [(i32, i32); 8] = [
    (0, 1),
    (0, -1),
    (1, 0),
    (-1, 0),
    (-1, -1),
    (1, -1),
    (-1, 1),
    (1, 1),
];

/// The nearest standable tile within chebyshev 1 of `edge.at` — the
/// take-off the transport interact must be sent from (the game only
/// accepts an `op_loc` from adjacent). `at` itself is the interact target
/// — a blocked loc tile — and is never a candidate.
/// [`scene_standable`] mirrors `WorldCollision::standable` against the
/// loaded scene's collision flags. `None` when no adjacent tile is
/// standable.
pub(super) fn approach_tile(
    snapshot: &GameSnapshot,
    at: WorldTile,
    here: WorldTile,
) -> Option<WorldTile> {
    APPROACH_RING
        .iter()
        .map(|(dx, dz)| WorldTile {
            x: at.x + dx,
            z: at.z + dz,
            level: at.level,
        })
        .filter(|t| scene_standable(snapshot, *t))
        .min_by_key(|t| cheb(*t, here))
}

/// Whether a tile is standable in the loaded scene: no footprint block —
/// no `WALK_SCENERY` footprint, no `WR_GRND` ground block, and no
/// `SQ_BLOCKED` base — the same test as `WorldCollision::standable`
/// against the client's raw collision flags. Tiles the scene has no flags
/// for (outside it, or on another level) are not standable.
pub(super) fn scene_standable(snapshot: &GameSnapshot, tile: WorldTile) -> bool {
    SceneQuery::new(snapshot.scene(), None)
        .collision_at(tile)
        .is_some_and(|flags| {
            flags
                & (CollisionFlag::WALK_SCENERY | CollisionFlag::WR_GRND | CollisionFlag::SQ_BLOCKED)
                == 0
        })
}

/// Chebyshev distance from `at` to the closest tile of `loc`'s rotated
/// footprint. A 1×1 loc equals origin distance; a length-6 ropeswing
/// whose origin is 4 tiles from `at` still matches when `at` sits on the
/// footprint. Does not widen the 3-tile search radius.
pub(super) fn loc_chebyshev_to_footprint(loc: &LocView, at: WorldTile) -> i32 {
    let fw = loc.footprint_width.max(1);
    let fl = loc.footprint_length.max(1);
    let min_x = loc.tile.x;
    let max_x = loc.tile.x + fw - 1;
    let min_z = loc.tile.z;
    let max_z = loc.tile.z + fl - 1;
    let dx = if at.x < min_x {
        min_x - at.x
    } else if at.x > max_x {
        at.x - max_x
    } else {
        0
    };
    let dz = if at.z < min_z {
        min_z - at.z
    } else if at.z > max_z {
        at.z - max_z
    } else {
        0
    };
    dx.max(dz)
}

/// The snapshot loc for a transport edge: the edge's closed `loc_id` or
/// `open_loc_id` on the edge's level within 3 tiles of `edge.at` measured
/// to the rotated footprint (the m8aq `gap <= 3`), nearest first.
/// Trapdoors `loc_change` closed→open (1568→1570); matching only the
/// closed id leaves Climb-down unarmed.
pub(super) fn find_transport_loc<'s>(
    snapshot: &'s GameSnapshot,
    edge: &TransportEdge,
) -> Option<&'s LocView> {
    snapshot
        .locs()
        .iter()
        .filter(|loc| {
            loc.tile.level == edge.at.level
                && (loc.id == edge.loc_id || edge.open_loc_id == Some(loc.id))
        })
        .map(|loc| (loc, loc_chebyshev_to_footprint(loc, edge.at)))
        .filter(|(_, gap)| *gap <= 3)
        .min_by_key(|(_, gap)| *gap)
        .map(|(loc, _)| loc)
}
