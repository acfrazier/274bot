//! Shared access geometry and air/routed selection for service footprints.

#[cfg(test)]
use api::service_pick::{ServiceCandidate, ServicePick};
use api::snapshot::WorldTile;

use crate::collision::{WorldCollision, WALL_EAST, WALL_NORTH, WALL_SOUTH, WALL_WEST};
use crate::named_banks::{AccessReach, Footprint};
use crate::router::{AvoidRect, FindOptions};
#[cfg(test)]
use crate::router::{find_first_with_avoid_budgets, FirstRouteSearch};
#[cfg(test)]
use crate::transport::TransportGraph;
#[cfg(test)]
use crate::world_state::WorldState;

/// World sides permitted for a service access tile. Bits run north, east,
/// south, west from least significant to most significant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Sides(u8);

impl Sides {
    pub(crate) const ALL: Self = Self(0x0f);

    /// Decode forceapproach's blocked-side mask, rotating clockwise by angle,
    /// then invert it to the open-side representation used by access geometry.
    pub(crate) const fn from_approach(packed: u8, angle: u8) -> Self {
        let turn = angle & 3;
        let mask = packed & 0x0f;
        let rotated = ((mask << turn) | (mask >> (4 - turn))) & 0x0f;
        Self(!rotated & 0x0f)
    }

    const fn allows(self, side: usize) -> bool {
        self.0 & (1 << side) != 0
    }
}

/// Standable access tiles outside `footprint`, filtered to allowed sides and
/// excluding a wall on the shared edge to the footprint tile touched.
pub(crate) fn service_access_tiles<'a>(
    collision: &'a WorldCollision,
    footprint: Footprint,
    reach: AccessReach,
    sides: Sides,
) -> impl Iterator<Item = WorldTile> + 'a {
    footprint
        .access_tiles(reach, |tile| collision.standable(tile))
        .filter(move |&tile| {
            let side = footprint_side(footprint, tile);
            sides.allows(side) && !wall_between(collision, tile, touched_tile(footprint, tile))
        })
}

fn footprint_side(footprint: Footprint, tile: WorldTile) -> usize {
    let west = tile.x < footprint.x;
    let east = tile.x >= footprint.x + footprint.width;
    let south = tile.z < footprint.z;
    let north = tile.z >= footprint.z + footprint.length;
    if north {
        0
    } else if east {
        1
    } else if south {
        2
    } else if west {
        3
    } else {
        0
    }
}

fn touched_tile(footprint: Footprint, tile: WorldTile) -> WorldTile {
    WorldTile {
        x: tile.x.clamp(footprint.x, footprint.x + footprint.width - 1),
        z: tile
            .z
            .clamp(footprint.z, footprint.z + footprint.length - 1),
        level: footprint.level,
    }
}

fn wall_between(collision: &WorldCollision, a: WorldTile, b: WorldTile) -> bool {
    if a.level != b.level {
        return false;
    }
    let (a_face, b_face) = match (b.x - a.x, b.z - a.z) {
        (0, 1) => (WALL_NORTH, WALL_SOUTH),
        (0, -1) => (WALL_SOUTH, WALL_NORTH),
        (1, 0) => (WALL_EAST, WALL_WEST),
        (-1, 0) => (WALL_WEST, WALL_EAST),
        _ => return false,
    };
    collision.walkable_word(a.x, a.z, a.level) & a_face != 0
        || collision.walkable_word(b.x, b.z, b.level) & b_face != 0
}

pub(crate) fn resolve_shop_keepers(
    data: &api::game_data::SelectedGameData,
    collision: &WorldCollision,
) -> api::service_pick::ShopKeeperFacts {
    api::service_pick::ShopKeeperFacts::resolve(data, |tile| {
        service_access_tiles(
            collision,
            Footprint {
                x: tile.x,
                z: tile.z,
                level: tile.level,
                width: 1,
                length: 1,
            },
            AccessReach::InLine,
            Sides::from_approach(0, 0),
        )
    })
}

pub(crate) fn air_rank(
    from: WorldTile,
    tile: WorldTile,
    width: u8,
    length: u8,
    access: WorldTile,
) -> (i32, i32, i32, i32, i32) {
    let dx = (tile.x - from.x)
        .max(from.x - (tile.x + i32::from(width) - 1))
        .max(0);
    let dz = (tile.z - from.z)
        .max(from.z - (tile.z + i32::from(length) - 1))
        .max(0);
    (
        i32::from(tile.level != from.level),
        dx.max(dz),
        (access.x - from.x).abs().max((access.z - from.z).abs()),
        access.x,
        access.z,
    )
}

/// Select the air-nearest access over borrowed, prevalidated candidates.
/// Ties preserve the bank rule: same level, footprint Chebyshev distance,
/// access Chebyshev distance, then access x and z.
///
/// Test-only ordering oracle: the host always routes the complete domain.
#[cfg(test)]
pub(crate) fn nearest_service_air<'a>(
    candidates: impl Iterator<Item = ServiceCandidate<'a>>,
    from: WorldTile,
) -> Option<ServicePick> {
    let mut best = None;
    let mut best_key = None;
    for candidate in candidates {
        let tile = candidate.tile;
        for &access_tile in candidate.access {
            let key = air_rank(from, tile, candidate.width, candidate.length, access_tile);
            if best_key.is_none_or(|current| key < current) {
                best_key = Some(key);
                best = Some(ServicePick {
                    index: candidate.index,
                    access_tile,
                });
            }
        }
    }
    best
}

/// Run one first-goal search over the complete target set, with teleports and
/// bank fetching disabled. The owner decides candidate eligibility before
/// constructing `targets`.
///
/// Test-only: the host submits [`routed_request`] to the cooperative job.
#[cfg(test)]
#[expect(
    clippy::too_many_arguments,
    reason = "Complete routed service-search inputs."
)]
pub(crate) fn nearest_service_routed(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    targets: &[WorldTile],
    mut opts: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
    budgets: (usize, usize),
) -> FirstRouteSearch {
    opts.allow_teleports = false;
    opts.allow_bank_fetch = false;
    opts.prefer_target_order = true;
    find_first_with_avoid_budgets(
        collision, graph, from, targets, opts, state, avoid, budgets.0, budgets.1,
    )
}

/// Build the cooperative request used by the host route job. Targets are
/// supplied as one complete owned set; this helper never pages or copies it.
pub fn routed_request(
    from: WorldTile,
    targets: Box<[WorldTile]>,
    mut options: FindOptions,
    avoid: Box<[AvoidRect]>,
    max_settles: usize,
) -> crate::router::SearchRequest {
    options.allow_teleports = false;
    options.allow_bank_fetch = false;
    options.prefer_target_order = true;
    crate::router::SearchRequest::First {
        from,
        targets,
        fallback: Box::default(),
        options,
        avoid,
        max_settles,
    }
}
