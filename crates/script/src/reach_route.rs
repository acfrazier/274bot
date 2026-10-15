//! Walking depth to an entity from the posted reach view.
//!
//! The host floods the live scene's collision from the player's tile
//! ([`api::query::SceneQuery::flood_reach`], packed by
//! [`api::query::pack_reach_query`]) and posts the dequeue ranks with the
//! step masks. A closed door is a wall in that collision, so an entity behind
//! one has no stand. [`route_steps`] reads the flood without running a
//! second one (the Quester target chooser); [`bounded_steps`] walks the
//! posted step masks inside a fixed box (`Shop.open`).

use api::query::ReachQueryView;
use api::WorldTile;
use std::collections::VecDeque;

/// The engine sends an NPC in a player's NPC info only while its south-west
/// tile is within this Chebyshev distance of the player
/// (`BuildArea.PREFERRED_VIEW_DISTANCE = 15`, `src/network/rsbuf/build.ts:57`;
/// checked by `CoordGrid.withinDistanceSw` at `build.ts:259` and
/// `info.ts:310`). Any NPC a script can see is inside this box.
pub(crate) const NPC_VIEW_DISTANCE: i32 = 15;

/// An entity's tiles: `width` × `length` from its south-west `tile`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Footprint {
    pub tile: WorldTile,
    pub width: i32,
    pub length: i32,
}

impl Footprint {
    pub(crate) fn tiles(self) -> impl Iterator<Item = WorldTile> {
        (0..self.width).flat_map(move |dx| {
            (0..self.length).map(move |dz| WorldTile {
                x: self.tile.x + dx,
                z: self.tile.z + dz,
                level: self.tile.level,
            })
        })
    }
}

/// The posted reach view when its flood starts on the live player tile.
/// A missing or stale flood leaves reachability unknown.
pub(crate) fn live(
    view: Option<&ReachQueryView>,
    here: Option<WorldTile>,
) -> Option<&ReachQueryView> {
    let view = view?;
    (exact_rank_at(view, here?) == Some(0)).then_some(view)
}

pub(crate) fn local_index(view: &ReachQueryView, tile: WorldTile) -> Option<usize> {
    let lx = tile.x - view.base_x;
    let lz = tile.z - view.base_z;
    (tile.level == view.level && lx >= 0 && lz >= 0 && lx < view.width && lz < view.height)
        .then(|| (lx as usize) * (view.height as usize) + (lz as usize))
}

pub(crate) fn reach_bit(view: &ReachQueryView, words: &[u32], tile: WorldTile) -> bool {
    ReachQueryView::bit_at(
        words,
        view.width,
        view.height,
        view.base_x,
        view.base_z,
        view.level,
        tile,
    )
}

fn exact_rank_at(view: &ReachQueryView, tile: WorldTile) -> Option<u16> {
    if !reach_bit(view, &view.reachable, tile) {
        return None;
    }
    let rank = *view.exact_rank.get(local_index(view, tile)?)?;
    (rank != u16::MAX).then_some(rank)
}

/// Best stand for a footprint: the earliest flood rank that is exact on, or
/// wall-valid orthogonally beside, a footprint tile. Returns its walking
/// depth and rank; `None` when the flood reaches no stand.
pub(crate) fn route_steps(view: &ReachQueryView, fp: Footprint) -> Option<(u32, u16)> {
    let (rank, stand) = fp
        .tiles()
        .filter_map(|tile| {
            if !reach_bit(view, &view.reachable_adj, tile) {
                return None;
            }
            let rank = *view.adjacent_rank.get(local_index(view, tile)?)?;
            (rank != u16::MAX).then(|| (rank, stand_for(view, tile, rank)))
        })
        .min_by_key(|(rank, _)| *rank)?;
    Some((
        flood_depth(view, stand, rank).unwrap_or(u32::from(rank)),
        rank,
    ))
}

/// The tile whose exact rank produced `rank`: the tile itself or one of its
/// orthogonal neighbours.
fn stand_for(view: &ReachQueryView, tile: WorldTile, rank: u16) -> WorldTile {
    [(0, 0), (-1, 0), (1, 0), (0, -1), (0, 1)]
        .into_iter()
        .map(|(dx, dz)| WorldTile {
            x: tile.x + dx,
            z: tile.z + dz,
            level: tile.level,
        })
        .find(|stand| exact_rank_at(view, *stand) == Some(rank))
        .unwrap_or(tile)
}

/// Walking depth of a flooded tile, recovered from dequeue ranks: each BFS
/// tile was discovered by its lowest-ranked neighbour able to step into it.
/// `None` when the posted step masks cannot reproduce the chain.
fn flood_depth(view: &ReachQueryView, mut tile: WorldTile, mut rank: u16) -> Option<u32> {
    let mut depth = 0u32;
    while rank != 0 {
        let (parent_rank, parent) = (-1..=1)
            .flat_map(|dx| (-1..=1).map(move |dz| (dx, dz)))
            .filter(|&(dx, dz)| dx != 0 || dz != 0)
            .filter_map(|(dx, dz)| {
                let from = WorldTile {
                    x: tile.x + dx,
                    z: tile.z + dz,
                    level: tile.level,
                };
                let from_rank = exact_rank_at(view, from)?;
                (from_rank < rank && view.can_step(from, tile)).then_some((from_rank, from))
            })
            .min_by_key(|(from_rank, _)| *from_rank)?;
        tile = parent;
        rank = parent_rank;
        depth += 1;
    }
    Some(depth)
}

/// One [`bounded_steps`] search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Bounded {
    /// Steps to the nearest stand, `None` when no stand is reachable inside
    /// the box.
    pub steps: Option<u32>,
    /// Tiles dequeued.
    pub expanded: usize,
    /// Largest Chebyshev distance from the start of a dequeued tile.
    pub farthest: i32,
}

/// Breadth-first search over the posted step masks
/// ([`ReachQueryView::can_step`]: the same per-tile collision the host
/// flood used, eight directions, diagonals only with both legs open) from
/// `here`, never leaving the Chebyshev box of `radius` around it, to the
/// nearest stand ([`reaches_npc_from`]). At most `(2 * radius + 1)^2` tiles
/// are dequeued.
pub(crate) fn bounded_steps(
    view: &ReachQueryView,
    here: WorldTile,
    fp: Footprint,
    radius: i32,
) -> Bounded {
    const DIRS: [(i32, i32); 8] = [
        (-1, 0),
        (1, 0),
        (0, -1),
        (0, 1),
        (-1, -1),
        (1, -1),
        (-1, 1),
        (1, 1),
    ];
    let radius = radius.max(0);
    let side = (2 * radius + 1) as usize;
    let slot = |tile: WorldTile| -> Option<usize> {
        let dx = tile.x - here.x + radius;
        let dz = tile.z - here.z + radius;
        (tile.level == here.level
            && (0..=2 * radius).contains(&dx)
            && (0..=2 * radius).contains(&dz))
        .then(|| dx as usize * side + dz as usize)
    };
    let inside = |tile: WorldTile| {
        tile.level == fp.tile.level
            && (fp.tile.x..fp.tile.x + fp.width).contains(&tile.x)
            && (fp.tile.z..fp.tile.z + fp.length).contains(&tile.z)
    };
    let mut seen = vec![false; side * side];
    let mut queue = VecDeque::new();
    let mut out = Bounded {
        steps: None,
        expanded: 0,
        farthest: 0,
    };
    if let Some(start) = slot(here) {
        seen[start] = true;
        queue.push_back((here, 0u32));
    }
    while let Some((tile, depth)) = queue.pop_front() {
        out.expanded += 1;
        out.farthest = out
            .farthest
            .max((tile.x - here.x).abs().max((tile.z - here.z).abs()));
        let stand = !inside(tile)
            && DIRS[..4].iter().any(|&(dx, dz)| {
                let npc = WorldTile {
                    x: tile.x + dx,
                    z: tile.z + dz,
                    level: tile.level,
                };
                inside(npc) && reaches_npc_from(view, tile, npc)
            });
        if stand {
            out.steps = Some(depth);
            return out;
        }
        for (dx, dz) in DIRS {
            let next = WorldTile {
                x: tile.x + dx,
                z: tile.z + dz,
                level: tile.level,
            };
            let Some(i) = slot(next) else {
                continue;
            };
            if !seen[i] && view.can_step(tile, next) {
                seen[i] = true;
                queue.push_back((next, depth + 1));
            }
        }
    }
    out
}

/// Whether a player on `stand` has reached the NPC tile `npc` cardinally
/// beside it: the engine's NPC op reach, rectangle-exclusive with wall flags
/// only (`PathingEntity.ts:408-435` -> `GameMap.ts:428-429` shape -2 ->
/// `ReachStrategy.ts:48-61` -> `RectangleBoundary.ts:15-27`). The stand must
/// have no wall on its side facing the NPC; the NPC's own ground does not
/// matter, and the NPC's tile is never a stand (the caller checks that).
///
/// The posted step masks hold that wall: the step from `npc` back onto
/// `stand` is open exactly when `stand` has neither the wall facing `npc`
/// nor a blocked square (`can_step_local`, `PL_WALK_*` = wall bit plus
/// `SQ_BLOCKED`), and it ignores `npc`'s own flags. Every stand the search
/// stepped onto has an open square, so for those this is the wall test. On
/// the start tile the client's flags may mark the square blocked; this then
/// answers false and the search looks for another stand: a longer route,
/// never a shorter one.
fn reaches_npc_from(view: &ReachQueryView, stand: WorldTile, npc: WorldTile) -> bool {
    view.can_step(npc, stand)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIDE: i32 = 80;

    /// An open field: every step from every tile is clear.
    fn field() -> ReachQueryView {
        let n = (SIDE * SIDE) as usize;
        ReachQueryView {
            available: true,
            base_x: 0,
            base_z: 0,
            level: 0,
            width: SIDE,
            height: SIDE,
            walkable: vec![u32::MAX; n.div_ceil(32)],
            reachable: vec![u32::MAX; n.div_ceil(32)],
            reachable_adj: vec![u32::MAX; n.div_ceil(32)],
            exact_rank: vec![0; n],
            adjacent_rank: vec![0; n],
            step: vec![u8::MAX; n],
            canlight: Vec::new(),
        }
    }

    fn tile(x: i32, z: i32) -> WorldTile {
        WorldTile { x, z, level: 0 }
    }

    fn npc(x: i32, z: i32) -> Footprint {
        Footprint {
            tile: tile(x, z),
            width: 1,
            length: 1,
        }
    }

    #[test]
    fn a_visible_keeper_in_the_open_is_its_walk_away() {
        let view = field();
        let here = tile(40, 40);
        // Ten tiles east: the stand west of it is nine steps away.
        let found = bounded_steps(&view, here, npc(50, 40), NPC_VIEW_DISTANCE);
        assert_eq!(found.steps, Some(9));
        // Diagonal: Chebyshev steps to the nearest cardinal stand (46, 47).
        let found = bounded_steps(&view, here, npc(47, 47), NPC_VIEW_DISTANCE);
        assert_eq!(found.steps, Some(7));
        // Already beside it.
        let found = bounded_steps(&view, here, npc(41, 40), NPC_VIEW_DISTANCE);
        assert_eq!(found.steps, Some(0));
    }

    #[test]
    fn the_search_never_expands_past_the_npc_view_box() {
        let view = field();
        let here = tile(40, 40);
        // A keeper the box cannot reach: every box tile is dequeued once,
        // none beyond the engine's NPC view distance.
        let found = bounded_steps(&view, here, npc(40 + 30, 40), NPC_VIEW_DISTANCE);
        let side = (2 * NPC_VIEW_DISTANCE + 1) as usize;
        assert_eq!(found.steps, None);
        assert_eq!(found.expanded, side * side);
        assert_eq!(found.farthest, NPC_VIEW_DISTANCE);
    }

    #[test]
    fn a_wall_with_no_way_round_inside_the_box_is_unreachable() {
        let mut view = field();
        // Close every step into or out of column x = 45 (a long wall).
        for z in 0..SIDE {
            for x in [44, 45, 46] {
                let i = (x * SIDE + z) as usize;
                view.step[i] = 0;
            }
        }
        let here = tile(40, 40);
        let found = bounded_steps(&view, here, npc(48, 40), NPC_VIEW_DISTANCE);
        assert_eq!(found.steps, None);
        assert!(found.farthest <= NPC_VIEW_DISTANCE);
    }

    // Client collision flags (`dash3d/collision_flag.rs`).
    const W_N: i32 = 0x2;
    const W_E: i32 = 0x8;
    const W_S: i32 = 0x20;
    const W_W: i32 = 0x80;
    /// `WALK_SCENERY` (a blocking loc) plus `WR_GRND` (blocked ground).
    const BLOCKED_GROUND: i32 = 0x100 | 0x200000;
    const ROOM: i32 = 31;

    /// An open scene whose posted view is packed the way the host packs it
    /// (`api::query::pack_step_masks`), so walls and blocked ground land in
    /// the step masks exactly as in a live view.
    struct Scene(api::snapshot::SceneView);

    impl Scene {
        fn open() -> Self {
            Self(api::snapshot::SceneView {
                available: true,
                base_x: 0,
                base_z: 0,
                level: 0,
                width: ROOM,
                height: ROOM,
                collision_flags: vec![0; (ROOM * ROOM) as usize],
            })
        }

        fn flag(&mut self, at: WorldTile, bits: i32) {
            self.0.collision_flags[(at.x * ROOM + at.z) as usize] |= bits;
        }

        /// A wall on the edge between cardinal neighbours `a` and `b`: the
        /// client sets the facing wall bit on both tiles.
        fn wall(&mut self, a: WorldTile, b: WorldTile) {
            let (toward_b, toward_a) = match (b.x - a.x, b.z - a.z) {
                (1, 0) => (W_E, W_W),
                (-1, 0) => (W_W, W_E),
                (0, 1) => (W_N, W_S),
                (0, -1) => (W_S, W_N),
                other => panic!("not cardinal neighbours: {other:?}"),
            };
            self.flag(a, toward_b);
            self.flag(b, toward_a);
        }

        fn view(&self) -> ReachQueryView {
            ReachQueryView {
                available: true,
                base_x: 0,
                base_z: 0,
                level: 0,
                width: ROOM,
                height: ROOM,
                walkable: Vec::new(),
                reachable: Vec::new(),
                reachable_adj: Vec::new(),
                exact_rank: Vec::new(),
                adjacent_rank: Vec::new(),
                step: api::query::pack_step_masks(&self.0),
                canlight: Vec::new(),
            }
        }
    }

    fn steps(view: &ReachQueryView, here: WorldTile, fp: Footprint) -> Option<u32> {
        bounded_steps(view, here, fp, NPC_VIEW_DISTANCE).steps
    }

    /// Review R3: the engine reaches an NPC from a cardinal stand with no
    /// wall facing it, whatever the NPC's own ground. Stepping onto that
    /// ground is closed, so the old step-onto-the-NPC test said unreachable.
    #[test]
    fn a_keeper_on_blocked_ground_is_reached_from_an_open_stand_beside_it() {
        let mut scene = Scene::open();
        let keeper = tile(12, 10);
        scene.flag(keeper, BLOCKED_GROUND);
        let view = scene.view();
        assert!(!view.can_step(tile(11, 10), keeper));
        assert_eq!(steps(&view, tile(11, 10), npc(12, 10)), Some(0));
        assert_eq!(steps(&view, tile(12, 11), npc(12, 10)), Some(0));
        assert_eq!(steps(&view, tile(8, 10), npc(12, 10)), Some(3));
    }

    /// A wall between the stand and the keeper (also a closed door: the
    /// client flags it as a wall) is no stand; the walk goes round to an
    /// open side. A closed room has no stand at all.
    #[test]
    fn a_separating_wall_or_closed_door_is_not_a_stand() {
        let mut scene = Scene::open();
        let keeper = tile(12, 10);
        scene.flag(keeper, BLOCKED_GROUND);
        scene.wall(tile(11, 10), keeper);
        let view = scene.view();
        // Beside it but walled off: round the corner to (12, 11). The
        // diagonal shortcut needs the walled leg, so it is two steps.
        assert_eq!(steps(&view, tile(11, 10), npc(12, 10)), Some(2));

        // A 3x3 room round the keeper, its door at (10, 10)-(11, 10) shut.
        let mut scene = Scene::open();
        scene.flag(keeper, BLOCKED_GROUND);
        for z in 9..=11 {
            scene.wall(tile(10, z), tile(11, z));
            scene.wall(tile(13, z), tile(14, z));
        }
        for x in 11..=13 {
            scene.wall(tile(x, 8), tile(x, 9));
            scene.wall(tile(x, 11), tile(x, 12));
        }
        let shut = scene.view();
        assert_eq!(steps(&shut, tile(8, 10), npc(12, 10)), None);
        // The door open: through it to the stand inside.
        let flags = &mut scene.0.collision_flags;
        flags[(10 * ROOM + 10) as usize] &= !W_E;
        flags[(11 * ROOM + 10) as usize] &= !W_W;
        assert_eq!(steps(&scene.view(), tile(8, 10), npc(12, 10)), Some(3));
    }

    /// The engine's reach is cardinal only: a diagonal tile is never a
    /// stand, even with no wall in the way.
    #[test]
    fn a_diagonal_tile_is_not_a_stand() {
        let mut scene = Scene::open();
        let keeper = tile(12, 10);
        scene.flag(keeper, BLOCKED_GROUND);
        let view = scene.view();
        assert_eq!(steps(&view, tile(11, 11), npc(12, 10)), Some(1));
        assert_eq!(steps(&view, tile(13, 9), npc(12, 10)), Some(1));
        // Every cardinal side walled, every diagonal open: no stand.
        for side in [tile(11, 10), tile(13, 10), tile(12, 9), tile(12, 11)] {
            scene.wall(side, keeper);
        }
        assert_eq!(steps(&scene.view(), tile(11, 11), npc(12, 10)), None);
    }

    /// The reach is exclusive: standing on the keeper (overlap) is not
    /// arrival, whether or not its ground is blocked; one step off is.
    #[test]
    fn standing_on_the_keeper_is_not_a_stand() {
        let view = Scene::open().view();
        assert_eq!(steps(&view, tile(12, 10), npc(12, 10)), Some(1));
        let mut scene = Scene::open();
        scene.flag(tile(12, 10), BLOCKED_GROUND);
        assert_eq!(steps(&scene.view(), tile(12, 10), npc(12, 10)), Some(1));
        // Inside a 2x2 keeper: one step out of its footprint.
        let big = Footprint {
            tile: tile(12, 10),
            width: 2,
            length: 2,
        };
        assert_eq!(steps(&view, tile(13, 11), big), Some(1));
        assert_eq!(steps(&view, tile(14, 11), big), Some(0));
        assert_eq!(steps(&view, tile(14, 12), big), Some(1));
    }
}
