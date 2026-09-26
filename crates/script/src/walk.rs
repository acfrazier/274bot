//! Frozen `Traversal.walkResilient` as a step machine, and the one native
//! walk both this family and `walk-hops` issue.
//!
//! Frozen (`Traversal.ts` 102–209, `walkLadder.ts` 47–86):
//! - `await Sustain.run()` at the start of each loop (`Traversal.ts:130`).
//! - `EventSignal.pending()` for an owned random (`ours`) returns false
//!   (`Traversal.ts:131–134`, `walkLadder.ts:48–52`). Guardian `hold` does
//!   not abort: `machine::on_hold` freezes the row and the walk pauses.
//! - `timeoutMs` is each baked `walkTo` bound, default 90_000 (`Traversal.ts:107`).
//! - `attempts` is `maxPasses` (`Traversal.ts:108`); the bound applies only
//!   when set (`Traversal.ts:149`).
//! - A closer Chebyshev rebakes and resets no-progress (`walkLadder.ts:55–59`).
//! - Frozen WalkExecutor returns true at `'closest'` (`WalkExecutor.ts:316-321`)
//!   even when `isArrived` is false. The resilient layer does not promote that
//!   low-level settlement to arrival; it continues `walkLadder`.
//! - No-progress after baked goes to the same-scene `DirectNavigator` step
//!   (`Traversal.ts:176–178`, `walkLadder.ts:73–77`). Its scene click clamps
//!   to 48 tiles, accepts the client's nearest reachable stand, and is
//!   reconsidered every two ticks / reissued after 2400 ms or a stall
//!   (`DirectNavigator.ts:15–25,28–51`; `ClientAdapter.ts:1739–1745`).
//! - No-progress after scene is the frozen unstick (`Traversal.ts:179–189`,
//!   `walkLadder.ts:79–86`): `tryNearbyDoor` then one `pickUnstickStep`.
//!   The posted loc page supplies candidates (`isOpenableBarrier`: name
//!   `/(door|gate)/i` and an `^Open` op, Chebyshev 3), excluding the frozen
//!   Desert Mining Camp scripted-door ids. Frozen path-scoped hints
//!   (`doorCrossing.ts:241–256, 264–321`) only keep hop/corridor doors when
//!   a `PathDoorHint` with tiles is supplied; off-path house doors are
//!   skipped. `walkResilient` calls `tryNearbyDoor(log)` with no path
//!   (`Traversal.ts:179`), so that filter does not apply here.
//!   Operator refinement (2026-09-25): only path-relevant doors — among
//!   those candidates, open the one whose cleared footprint makes `dest`
//!   reachable under the walk arrival rule (else strictly reduces the
//!   best reachable Chebyshev), tie-break shortest route then nearest.
//!   An irrelevant door is not opened, but the frozen one-tile unstick
//!   step still runs before the pass is counted. Unlike frozen, the native
//!   wait does not dismiss a quest-lock mesbox, and a locked door remains
//!   shut for the full 5-second bound.
//! - `UNREACHABLE_PASSES` (3) then verify (`walkLadder.ts:33, 83–84`). No
//!   `WalkExecutor.probeDest` here, so verify is fail-closed as probe-dead
//!   (`walkLadder.ts:66–68`).
//! - Backoff 2–16 ticks (`walkLadder.ts:30–31, 39–40`).

//! - Frozen `WalkExecutor.lastOutcome === 'blocked'` returns true
//!   (`Traversal.ts:172–174`). This host's walk wait is arrived-or-failed;
//!   there is no blocked, so a settled walk is re-checked with `isArrived`.
//! - Frozen `budget` rebakes once with `bigBudget` (`walkLadder.ts:73–75`).
//!   The wait is a bool; there is no budget vs failed, so no big-budget rebake.

use crate::machine::{Begin, Call, Cx, Family, Reply, Step};
use crate::observed::{self, SceneRow};
use crate::shim::InteractReq;
use crate::walk_wait;
use api::line_of_sight::CollisionQuery;
use api::query::{SceneQuery, SceneReachOptions};
use api::snapshot::{SceneView, WorldTile};
use serde::Deserialize;
use serde_json::json;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Frozen `opts.timeoutMs ?? 90000` (`Traversal.ts:107`).
pub(crate) const BAKED_TIMEOUT_MS: u64 = 90_000;
/// Frozen `SCENE_TIMEOUT_MS` (`Traversal.ts:37,176–178`).
const SCENE_TIMEOUT_MS: u64 = 6_000;
/// Frozen direct-scene re-click interval (`DirectNavigator.ts:43–47`).
const SCENE_RECLICK_MS: u64 = 2_400;
const SCENE_CHECK_TICKS: u8 = 2;
const SCENE_CLAMP_TILES: i32 = 48;
/// Frozen `walkWithHops` / hop approach `attempts: 3`.
pub(crate) const HOP_ATTEMPTS: u32 = 3;
/// Frozen `UNREACHABLE_PASSES` (`walkLadder.ts:33`).
const UNREACHABLE_PASSES: u32 = 3;
const BACKOFF_MIN: u32 = 2;
const BACKOFF_MAX: u32 = 16;
/// Frozen `tryNearbyDoor` wait (`doorCrossing.ts:369–380`).
const UNSTICK_DOOR_MS: u64 = 5_000;
/// Frozen `pickUnstickStep` DirectNavigator bound (`Traversal.ts:186`).
const UNSTICK_STEP_MS: u64 = 3_000;
/// Frozen `Locs.query().within(3)` (`doorCrossing.ts:316–322`).
const UNSTICK_RADIUS: i32 = 3;
/// Frozen `DESERT_MINING_CAMP_SCRIPTED_DOOR_IDS`
/// (`doorCrossing.ts:330–333`, `desertMiningCampDoors.ts:1–3`).
const DESERT_MINING_CAMP_SCRIPTED_DOOR_IDS: [i32; 8] =
    [2673, 2674, 2675, 2676, 2687, 2688, 2690, 2691];
/// Frozen `DIRS` (`walkLadder.ts:90–93`): N, NE, E, SE, S, SW, W, NW.
const UNSTICK_DIRS: [(i32, i32); 8] = [
    (0, 1),
    (1, 1),
    (1, 0),
    (1, -1),
    (0, -1),
    (-1, -1),
    (-1, 0),
    (-1, 1),
];
/// Frozen `walkOpening` segment cap (`walkOpening.ts:104`).
const OPENING_SEGMENTS: u32 = 8;
/// Frozen `ESCAPE_RADIUS` (`walkOpening.ts:15`).
const ESCAPE_RADIUS: i32 = 14;
/// Frozen `OPEN_WAIT_MS` (`walkOpening.ts:18`).
const OPEN_WAIT_MS: u64 = 4_000;
/// Frozen approach `walkTo` bound (`walkOpening.ts:136`).
const OPENING_DOOR_WALK_MS: u64 = 45_000;

#[cfg(test)]
thread_local! {
    static SCENE_TEST_CLOCK: std::cell::Cell<Option<(Instant, Instant)>> =
        const { std::cell::Cell::new(None) };
}

fn scene_now(cx: &mut Cx<'_>) -> Instant {
    #[cfg(test)]
    if let Some((_, now)) = SCENE_TEST_CLOCK.with(std::cell::Cell::get) {
        return now;
    }
    cx.clock().now()
}

#[cfg(test)]
fn reset_scene_time() {
    let now = Instant::now();
    SCENE_TEST_CLOCK.with(|clock| clock.set(Some((now, now))));
}

#[cfg(test)]
fn age_scene_time(millis: u64) {
    SCENE_TEST_CLOCK.with(|clock| {
        let (base, _) = clock.get().expect("scene test clock initialized");
        clock.set(Some((base, base + Duration::from_millis(millis))));
    });
}

#[derive(Clone, Copy, Debug, Deserialize)]
struct Tile {
    x: i32,
    z: i32,
    #[serde(default)]
    level: i32,
}

impl Tile {
    fn world(self) -> WorldTile {
        WorldTile {
            x: self.x,
            z: self.z,
            level: self.level,
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WalkResilientOpts {
    #[serde(default)]
    radius: i32,
    #[serde(default)]
    attempts: Option<u32>,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    use_teleport_catalog: bool,
}

#[derive(Deserialize)]
pub(crate) struct WalkResilientArgs {
    tile: Tile,
    #[serde(default)]
    opts: WalkResilientOpts,
}

/// Frozen `walkChebyshev`: Chebyshev on the plane, never close across a floor.
fn walk_chebyshev(a: WorldTile, b: WorldTile) -> i32 {
    let xz = (a.x - b.x).abs().max((a.z - b.z).abs());
    if a.level == b.level {
        xz
    } else {
        1_000_000 + xz
    }
}

fn here() -> Option<WorldTile> {
    observed::with(|scene| {
        scene.since_login().here().map(|t| WorldTile {
            x: t.x,
            z: t.z,
            level: t.level,
        })
    })
}

/// Frozen `EventSignal.pending()` for an owned random. Guardian `hold`
/// freezes the row (`machine::on_hold`) instead of aborting the walk.
fn interrupted() -> bool {
    observed::with(|scene| scene.since_login().ours().unwrap_or(false))
}

/// Frozen `isArrived` over the cached reach view — the same helper
/// `walk_wait` uses (`posted_here` + flood origin + `canReachAdjacent`).
fn arrived(dest: WorldTile, radius: i32) -> bool {
    crate::load::reach_query::arrived(dest, radius)
}

/// Frozen `backoffTicks` (`walkLadder.ts:39–40`).
fn backoff_ticks(no_progress_passes: u32) -> u32 {
    BACKOFF_MAX.min(BACKOFF_MIN + 2 * no_progress_passes.saturating_sub(1))
}

fn locs() -> Vec<SceneRow> {
    observed::with(|scene| scene.since_login().locs().cloned().unwrap_or_default())
}

fn loc_world(loc: &SceneRow) -> WorldTile {
    WorldTile {
        x: loc.x,
        z: loc.z,
        level: loc.level,
    }
}

/// Posted ops, without empty and `hidden` slots (frozen `actions()`).
fn loc_ops(loc: &SceneRow) -> impl Iterator<Item = &str> {
    loc.actions
        .iter()
        .map(|op| &**op)
        .filter(|op| !op.is_empty() && *op != "hidden")
}

fn op_starting(loc: &SceneRow, prefix: &str) -> Option<String> {
    loc_ops(loc)
        .find(|op| {
            op.get(..prefix.len())
                .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
        })
        .map(str::to_string)
}

/// Frozen `isOpenableBarrier` (`doorCrossing.ts:190–192`).
fn openable_barrier(loc: &SceneRow) -> bool {
    let name = loc.name_or_empty().to_ascii_lowercase();
    (name.contains("door") || name.contains("gate")) && op_starting(loc, "open").is_some()
}

fn posted_collision() -> Option<CollisionQuery> {
    observed::with(|scene| scene.since_login().collision().cloned())
}

fn scene_from_collision(c: &CollisionQuery) -> Option<SceneView> {
    if !c.available || c.width <= 0 || c.height <= 0 {
        return None;
    }
    Some(SceneView {
        available: true,
        base_x: c.base_x,
        base_z: c.base_z,
        level: c.level,
        width: c.width,
        height: c.height,
        collision_flags: c.flags.to_vec(),
    })
}

fn clear_collision_bits(scene: &mut SceneView, x: i32, z: i32, bits: i32) {
    let lx = x - scene.base_x;
    let lz = z - scene.base_z;
    if lx < 0 || lz < 0 || lx >= scene.width || lz >= scene.height {
        return;
    }
    let i = lx as usize * scene.height as usize + lz as usize;
    if let Some(slot) = scene.collision_flags.get_mut(i) {
        *slot &= !bits;
    }
}

fn clear_door_tile(scene: &mut SceneView, door: &SceneRow) {
    // Client `CollisionMap.add_wall` / `del_wall` and `LocShape` /
    // `LocAngle` (`collision_map.rs:186–232,283–329`).
    const WALL_STRAIGHT: i32 = 0;
    const WALL_DIAGONAL_CORNER: i32 = 1;
    const WALL_L: i32 = 2;
    const WALL_SQUARE_CORNER: i32 = 3;
    const WEST: i32 = 0;
    const NORTH: i32 = 1;
    const EAST: i32 = 2;
    const SOUTH: i32 = 3;
    const NORTH_WEST: i32 = 0x1;
    const NORTH_EDGE: i32 = 0x2;
    const NORTH_EAST: i32 = 0x4;
    const EAST_EDGE: i32 = 0x8;
    const SOUTH_EAST: i32 = 0x10;
    const SOUTH_EDGE: i32 = 0x20;
    const SOUTH_WEST: i32 = 0x40;
    const WEST_EDGE: i32 = 0x80;

    if door.level != scene.level {
        return;
    }
    let (x, z) = (door.x, door.z);
    match (i32::from(door.shape), i32::from(door.angle)) {
        (WALL_STRAIGHT, WEST) => {
            clear_collision_bits(scene, x, z, WEST_EDGE);
            clear_collision_bits(scene, x - 1, z, EAST_EDGE);
        }
        (WALL_STRAIGHT, NORTH) => {
            clear_collision_bits(scene, x, z, NORTH_EDGE);
            clear_collision_bits(scene, x, z + 1, SOUTH_EDGE);
        }
        (WALL_STRAIGHT, EAST) => {
            clear_collision_bits(scene, x, z, EAST_EDGE);
            clear_collision_bits(scene, x + 1, z, WEST_EDGE);
        }
        (WALL_STRAIGHT, SOUTH) => {
            clear_collision_bits(scene, x, z, SOUTH_EDGE);
            clear_collision_bits(scene, x, z - 1, NORTH_EDGE);
        }
        (WALL_DIAGONAL_CORNER | WALL_SQUARE_CORNER, WEST) => {
            clear_collision_bits(scene, x, z, NORTH_WEST);
            clear_collision_bits(scene, x - 1, z + 1, SOUTH_EAST);
        }
        (WALL_DIAGONAL_CORNER | WALL_SQUARE_CORNER, NORTH) => {
            clear_collision_bits(scene, x, z, NORTH_EAST);
            clear_collision_bits(scene, x + 1, z + 1, SOUTH_WEST);
        }
        (WALL_DIAGONAL_CORNER | WALL_SQUARE_CORNER, EAST) => {
            clear_collision_bits(scene, x, z, SOUTH_EAST);
            clear_collision_bits(scene, x + 1, z - 1, NORTH_WEST);
        }
        (WALL_DIAGONAL_CORNER | WALL_SQUARE_CORNER, SOUTH) => {
            clear_collision_bits(scene, x, z, SOUTH_WEST);
            clear_collision_bits(scene, x - 1, z - 1, NORTH_EAST);
        }
        (WALL_L, WEST) => {
            clear_collision_bits(scene, x, z, NORTH_EDGE | WEST_EDGE);
            clear_collision_bits(scene, x - 1, z, EAST_EDGE);
            clear_collision_bits(scene, x, z + 1, SOUTH_EDGE);
        }
        (WALL_L, NORTH) => {
            clear_collision_bits(scene, x, z, NORTH_EDGE | EAST_EDGE);
            clear_collision_bits(scene, x, z + 1, SOUTH_EDGE);
            clear_collision_bits(scene, x + 1, z, WEST_EDGE);
        }
        (WALL_L, EAST) => {
            clear_collision_bits(scene, x, z, SOUTH_EDGE | EAST_EDGE);
            clear_collision_bits(scene, x + 1, z, WEST_EDGE);
            clear_collision_bits(scene, x, z - 1, NORTH_EDGE);
        }
        (WALL_L, SOUTH) => {
            clear_collision_bits(scene, x, z, SOUTH_EDGE | WEST_EDGE);
            clear_collision_bits(scene, x, z - 1, NORTH_EDGE);
            clear_collision_bits(scene, x - 1, z, EAST_EDGE);
        }
        _ => {
            let lx = x - scene.base_x;
            let lz = z - scene.base_z;
            if lx >= 0 && lz >= 0 && lx < scene.width && lz < scene.height {
                let i = lx as usize * scene.height as usize + lz as usize;
                if let Some(slot) = scene.collision_flags.get_mut(i) {
                    *slot = 0;
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
struct FloodScore {
    routable: bool,
    route_depth: u16,
    best_cheb: i32,
    best_depth: u16,
}

fn flood_score(
    scene: &SceneView,
    me: WorldTile,
    dest: WorldTile,
    radius: i32,
) -> Option<FloodScore> {
    if !scene.available || me.level != scene.level || dest.level != scene.level {
        return None;
    }
    let width = usize::try_from(scene.width).ok()?;
    let height = usize::try_from(scene.height).ok()?;
    let count = width.checked_mul(height)?;
    if count == 0 {
        return None;
    }
    let start_x = me.x - scene.base_x;
    let start_z = me.z - scene.base_z;
    if start_x < 0 || start_z < 0 || start_x >= scene.width || start_z >= scene.height {
        return None;
    }

    let query = SceneQuery::new(scene, Some(me));
    let mut depths = vec![u16::MAX; count];
    let start = start_x as usize * height + start_z as usize;
    depths[start] = 0;
    let mut queue = VecDeque::from([me]);
    let mut route_depth = u16::MAX;
    let mut best_cheb = i32::MAX;
    let mut best_depth = u16::MAX;
    while let Some(tile) = queue.pop_front() {
        let lx = tile.x - scene.base_x;
        let lz = tile.z - scene.base_z;
        let i = lx as usize * height + lz as usize;
        let depth = depths[i];
        let cheb = walk_chebyshev(tile, dest);
        if cheb <= radius {
            route_depth = route_depth.min(depth);
        }
        if cheb < best_cheb {
            best_cheb = cheb;
            best_depth = depth;
        } else if cheb == best_cheb {
            best_depth = best_depth.min(depth);
        }

        for (dx, dz) in UNSTICK_DIRS {
            let next = WorldTile {
                x: tile.x + dx,
                z: tile.z + dz,
                level: tile.level,
            };
            let nx = next.x - scene.base_x;
            let nz = next.z - scene.base_z;
            if nx < 0 || nz < 0 || nx >= scene.width || nz >= scene.height {
                continue;
            }
            let ni = nx as usize * height + nz as usize;
            if depths[ni] != u16::MAX || !query.can_step(tile, next) {
                continue;
            }
            depths[ni] = depth.saturating_add(1);
            queue.push_back(next);
        }
    }
    let reached = |tile: WorldTile| {
        let lx = tile.x - scene.base_x;
        let lz = tile.z - scene.base_z;
        lx >= 0
            && lz >= 0
            && lx < scene.width
            && lz < scene.height
            && depths[lx as usize * height + lz as usize] != u16::MAX
    };
    let arrival_reachable = if query.walkable(dest) {
        reached(dest)
    } else {
        query.arrival_stands(dest).any(reached)
    };
    if !arrival_reachable {
        route_depth = u16::MAX;
    }
    Some(FloodScore {
        routable: route_depth != u16::MAX,
        route_depth,
        best_cheb,
        best_depth,
    })
}

/// Operator refinement: only a door whose opening helps this walk.
fn pick_helpful_door(
    me: WorldTile,
    dest: WorldTile,
    radius: i32,
    candidates: Vec<SceneRow>,
) -> Option<SceneRow> {
    if candidates.is_empty() {
        return None;
    }
    let collision = posted_collision()?;
    let base_scene = scene_from_collision(&collision)?;
    let base = flood_score(&base_scene, me, dest, radius)?;
    let mut best: Option<(SceneRow, bool, i32, u16, i32)> = None;
    for door in candidates {
        let tile = loc_world(&door);
        let mut opened = base_scene.clone();
        clear_door_tile(&mut opened, &door);
        let Some(score) = flood_score(&opened, me, dest, radius) else {
            continue;
        };
        let helps = if score.routable {
            !base.routable || score.route_depth < base.route_depth
        } else {
            !base.routable && score.best_cheb < base.best_cheb
        };
        if !helps {
            continue;
        }
        let nearest = walk_chebyshev(me, tile);
        let distance = if score.routable { 0 } else { score.best_cheb };
        let depth = if score.routable {
            score.route_depth
        } else {
            score.best_depth
        };
        let better =
            best.as_ref()
                .is_none_or(|(_, was_routable, was_distance, was_depth, was_near)| {
                    (
                        score.routable,
                        std::cmp::Reverse(distance),
                        std::cmp::Reverse(depth),
                        std::cmp::Reverse(nearest),
                    ) > (
                        *was_routable,
                        std::cmp::Reverse(*was_distance),
                        std::cmp::Reverse(*was_depth),
                        std::cmp::Reverse(*was_near),
                    )
                });
        if better {
            best = Some((door, score.routable, distance, depth, nearest));
        }
    }
    best.map(|(door, _, _, _, _)| door)
}

fn unstick_candidates(me: WorldTile) -> Vec<SceneRow> {
    locs()
        .into_iter()
        .filter(|loc| {
            !DESERT_MINING_CAMP_SCRIPTED_DOOR_IDS.contains(&loc.id)
                && openable_barrier(loc)
                && walk_chebyshev(me, loc_world(loc)) <= UNSTICK_RADIUS
        })
        .collect()
}

fn barrier_still_shut(tile: WorldTile, name: &str) -> bool {
    locs().into_iter().any(|loc| {
        loc.x == tile.x && loc.z == tile.z && loc.name_or_empty() == name && openable_barrier(&loc)
    })
}

fn can_reach_adjacent(tile: WorldTile, max_steps: Option<u32>) -> bool {
    crate::load::reach_query::with_view(|view| {
        view.can_reach(
            tile,
            &SceneReachOptions {
                max_steps,
                adjacent_ok: true,
            },
        )
    })
}

fn can_step(from: WorldTile, to: WorldTile) -> bool {
    crate::load::reach_query::with_view(|view| view.can_step(from, to))
}

/// Frozen `pickUnstickStep` (`walkLadder.ts:96–104`).
fn pick_unstick_step(me: WorldTile, start_dir: u8) -> Option<(i32, i32)> {
    for i in 0..UNSTICK_DIRS.len() {
        let (dx, dz) = UNSTICK_DIRS[(usize::from(start_dir) + i) % UNSTICK_DIRS.len()];
        let to = WorldTile {
            x: me.x + dx,
            z: me.z + dz,
            level: me.level,
        };
        if can_step(me, to) {
            return Some((dx, dz));
        }
    }
    None
}

fn emit_loc_open(loc: &SceneRow, op: &str, cx: &mut Cx<'_>) {
    cx.emit(InteractReq::Loc {
        x: loc.x,
        z: loc.z,
        level: loc.level,
        action: op.to_string(),
        id: Some(loc.id),
    });
}

/// Frozen `isOpenableObstacle` (`walkOpening.ts:79–82`).
fn openable_obstacle(loc: &SceneRow, obstacles: &[String]) -> bool {
    let name = loc.name_or_empty().to_ascii_lowercase();
    obstacles
        .iter()
        .any(|key| !key.is_empty() && name.contains(&key.to_ascii_lowercase()))
        && op_starting(loc, "open").is_some()
}

fn pick_opening_door(
    me: WorldTile,
    dest: WorldTile,
    radius: i32,
    obstacles: &[String],
) -> Option<SceneRow> {
    let in_range: Vec<SceneRow> = locs()
        .into_iter()
        .filter(|loc| {
            openable_obstacle(loc, obstacles)
                && walk_chebyshev(me, loc_world(loc)) <= ESCAPE_RADIUS
                && can_reach_adjacent(loc_world(loc), None)
        })
        .collect();
    pick_helpful_door(me, dest, radius, in_range)
}

fn shut_obstacle_at(door: WorldTile, obstacles: &[String]) -> Option<SceneRow> {
    locs()
        .into_iter()
        .filter(|loc| loc.x == door.x && loc.z == door.z && openable_obstacle(loc, obstacles))
        .min_by_key(|loc| loc.distance)
}

/// One native walk: arrival check, then the settled outcome. Frozen
/// `walkResilient`'s baked `walkTo`; `walk-hops` and `walk-resilient` share it.
pub(crate) struct Walk {
    token: u64,
}

impl Walk {
    /// `Err(arrived)` when no walk was needed (or no tile is posted).
    pub(crate) fn begin(
        dest: WorldTile,
        radius: i32,
        timeout_ms: u64,
        allow_teleports: bool,
        cx: &mut Cx<'_>,
    ) -> Result<Self, bool> {
        if here().is_none() {
            return Err(false);
        }

        if crate::load::reach_query::arrived(dest, radius) {
            return Err(true);
        }

        let token = walk_wait::dispatch(&json!({
            "op": "begin",
            "x": dest.x,
            "z": dest.z,
            "level": dest.level,
            "radius": radius,
            "allow_teleports": allow_teleports,
        }))
        .as_u64()
        .unwrap_or(0);
        cx.emit(if radius > 0 {
            InteractReq::WalkNear {
                x: dest.x,
                z: dest.z,
                level: dest.level,
                radius,
                allow_teleports,
                allow_wilderness: true,
                allow_bank_fetch: true,
                request_id: token,
            }
        } else {
            InteractReq::Walk {
                x: dest.x,
                z: dest.z,
                level: dest.level,
                allow_teleports,
                allow_wilderness: true,
                allow_bank_fetch: true,
                request_id: token,
            }
        });
        cx.clock().arm(timeout_ms);
        Ok(Self { token })
    }

    /// `Some(settled)` once the walk wait settled or timed out. A timeout
    /// stops the host follow (as a returned frozen `walkResilient` has
    /// stopped its walker). The bool is the wait's value, not `isArrived`:
    /// a closest terminal is true here and the ladder re-checks arrival.
    pub(crate) fn step(&self, cx: &mut Cx<'_>) -> Option<bool> {
        let settled = walk_wait::dispatch(&json!({ "op": "settled", "token": self.token }))
            .as_bool()
            .unwrap_or(false);
        if settled {
            return Some(
                walk_wait::dispatch(&json!({ "op": "value", "token": self.token }))
                    .as_bool()
                    .unwrap_or(false),
            );
        }
        if !cx.clock().bound_reached() {
            return None;
        }
        self.abort(cx);
        Some(false)
    }

    pub(crate) fn abort(&self, cx: &mut Cx<'_>) {
        cx.emit(InteractReq::AbortWalk {
            request_id: self.token,
        });
    }
}

struct SceneStep {
    last_issued: Instant,
    last_tile: WorldTile,
    delay_left: u8,
}

struct UnstickStep {
    target: WorldTile,
    last_issued: Instant,
    last_tile: WorldTile,
    delay_left: u8,
}

impl SceneStep {
    fn should_reclick(&self, current: WorldTile, now: Instant) -> bool {
        current == self.last_tile
            || now.saturating_duration_since(self.last_issued)
                >= Duration::from_millis(SCENE_RECLICK_MS)
    }
}

enum Phase {
    NeedWalk,
    Walking(Walk),
    Scene(SceneStep),
    UnstickDoor { tile: WorldTile, name: String },
    UnstickStep(UnstickStep),
    Backoff,
}

/// Frozen walkResilient ladder over baked [`Walk`]s. Shared by `walk-hops`.
pub(crate) struct Resilient {
    dest: WorldTile,
    radius: i32,
    timeout_ms: u64,
    attempts: Option<u32>,
    allow_teleports: bool,
    phase: Phase,
    best_dist: i32,
    no_progress: u32,
    delay_left: u32,
    unstick_dir: u8,
    logs: VecDeque<String>,
}

impl Resilient {
    pub(crate) fn new(
        dest: WorldTile,
        radius: i32,
        timeout_ms: u64,
        attempts: Option<u32>,
        allow_teleports: bool,
    ) -> Self {
        let best_dist = here().map(|h| walk_chebyshev(h, dest)).unwrap_or(i32::MAX);
        Self {
            dest,
            radius,
            timeout_ms,
            attempts,
            allow_teleports,
            phase: Phase::NeedWalk,
            best_dist,
            no_progress: 0,
            delay_left: 0,
            unstick_dir: 0,
            logs: VecDeque::new(),
        }
    }

    /// Issue the first baked walk. `Err(done)` if no walk was needed.
    pub(crate) fn start(mut self, cx: &mut Cx<'_>) -> Result<Self, bool> {
        if interrupted() {
            self.logs
                .push_back("walk interrupted by a runtime event — yielding to the runtime".into());
            return Err(false);
        }
        let Some(_) = here() else {
            return Err(false);
        };
        if arrived(self.dest, self.radius) {
            return Err(true);
        }
        match self.kick_walk(cx) {
            None => Ok(self),
            Some(done) => Err(done),
        }
    }

    pub(crate) fn pop_log(&mut self) -> Option<String> {
        self.logs.pop_front()
    }

    /// `Some(result)` once the ladder ended; `None` waits.
    pub(crate) fn step(&mut self, cx: &mut Cx<'_>) -> Option<bool> {
        // Frozen: EventSignal.pending before isArrived (Traversal.ts:131,
        // walkLadder.ts:48–52).
        if interrupted() {
            if let Phase::Walking(walk) = &self.phase {
                walk.abort(cx);
            }
            self.logs
                .push_back("walk interrupted by a runtime event — yielding to the runtime".into());
            return Some(false);
        }
        if arrived(self.dest, self.radius) {
            return Some(true);
        }
        if let Some(max) = self.attempts {
            if self.no_progress >= max {
                self.logs.push_back(format!(
                    "walkResilient: {max} passes made no progress — stopping (bounded caller)"
                ));
                return Some(false);
            }
        }
        match std::mem::replace(&mut self.phase, Phase::NeedWalk) {
            Phase::NeedWalk => self.kick_walk(cx),
            Phase::Scene(mut scene) => {
                if arrived(self.dest, self.radius.saturating_add(1)) || cx.clock().bound_reached() {
                    self.after_scene(cx)
                } else if scene.delay_left > 1 {
                    scene.delay_left -= 1;
                    self.phase = Phase::Scene(scene);
                    None
                } else {
                    let Some(current) = here() else {
                        return Some(false);
                    };
                    let now = scene_now(cx);
                    if scene.should_reclick(current, now) {
                        self.emit_scene_walk(current, cx);
                        scene.last_issued = now;
                    }
                    scene.last_tile = current;
                    scene.delay_left = SCENE_CHECK_TICKS;
                    self.phase = Phase::Scene(scene);
                    None
                }
            }
            Phase::UnstickDoor { tile, name } => {
                // Frozen ORs `canReach adjacent` (`doorCrossing.ts:369–380`).
                // A closed door the player already stands next to is adjacent-
                // reachable, so that clause would skip the wait and the Open
                // would not land. Wait for the shut leaf to leave, else the
                // 5000 ms bound.
                if !barrier_still_shut(tile, &name) || cx.clock().bound_reached() {
                    self.kick_unstick_step(cx)
                } else {
                    self.phase = Phase::UnstickDoor { tile, name };
                    None
                }
            }
            Phase::UnstickStep(mut step) => {
                let Some(current) = here() else {
                    return Some(false);
                };
                if current == step.target || cx.clock().bound_reached() {
                    self.after_unstick_step(cx)
                } else if step.delay_left > 1 {
                    step.delay_left -= 1;
                    self.phase = Phase::UnstickStep(step);
                    None
                } else {
                    let now = scene_now(cx);
                    if current == step.last_tile
                        || now.saturating_duration_since(step.last_issued)
                            >= Duration::from_millis(SCENE_RECLICK_MS)
                    {
                        cx.emit(InteractReq::WalkTo {
                            x: step.target.x,
                            z: step.target.z,
                            level: step.target.level,
                        });
                        step.last_issued = now;
                    }
                    step.last_tile = current;
                    step.delay_left = SCENE_CHECK_TICKS;
                    self.phase = Phase::UnstickStep(step);
                    None
                }
            }
            Phase::Backoff => {
                if self.delay_left > 1 {
                    self.delay_left -= 1;
                    self.phase = Phase::Backoff;
                    None
                } else {
                    self.delay_left = 0;
                    self.kick_walk(cx)
                }
            }
            Phase::Walking(walk) => {
                let owns_wait = walk_wait::owns(walk.token);
                match walk.step(cx) {
                    None => {
                        self.phase = Phase::Walking(walk);
                        None
                    }
                    Some(_) if owns_wait => self.after_baked(cx),
                    Some(_) => self.after_displaced_walk(),
                }
            }
        }
    }

    fn kick_walk(&mut self, cx: &mut Cx<'_>) -> Option<bool> {
        match Walk::begin(
            self.dest,
            self.radius,
            self.timeout_ms,
            self.allow_teleports,
            cx,
        ) {
            Ok(walk) => {
                self.phase = Phase::Walking(walk);
                None
            }
            Err(done) => Some(done),
        }
    }

    fn scene_walk_target(&self, current: WorldTile) -> WorldTile {
        let clamp = |value: i32, around: i32| {
            value.clamp(
                around.saturating_sub(SCENE_CLAMP_TILES),
                around.saturating_add(SCENE_CLAMP_TILES),
            )
        };
        WorldTile {
            x: clamp(self.dest.x, current.x),
            z: clamp(self.dest.z, current.z),
            level: current.level,
        }
    }

    fn emit_scene_walk(&self, current: WorldTile, cx: &mut Cx<'_>) {
        let target = self.scene_walk_target(current);
        cx.emit(InteractReq::WalkTo {
            x: target.x,
            z: target.z,
            level: target.level,
        });
    }

    fn kick_scene(&mut self, cx: &mut Cx<'_>) -> Option<bool> {
        let scene_radius = self.radius.saturating_add(1);
        if arrived(self.dest, scene_radius) {
            return self.after_scene(cx);
        }
        let Some(current) = here() else {
            return Some(false);
        };
        self.emit_scene_walk(current, cx);
        let last_issued = scene_now(cx);
        cx.clock().arm(SCENE_TIMEOUT_MS);
        self.phase = Phase::Scene(SceneStep {
            last_issued,
            last_tile: current,
            delay_left: SCENE_CHECK_TICKS,
        });
        None
    }

    fn after_baked(&mut self, cx: &mut Cx<'_>) -> Option<bool> {
        // Frozen next-loop order: pending, then withinRadius (N5/N6).
        if interrupted() {
            self.logs
                .push_back("walk interrupted by a runtime event — yielding to the runtime".into());
            return Some(false);
        }
        if arrived(self.dest, self.radius) {
            return Some(true);
        }
        let Some(here) = here() else {
            return Some(false);
        };
        let cur = walk_chebyshev(here, self.dest);
        if cur < self.best_dist {
            self.best_dist = cur;
            self.no_progress = 0;
            return self.kick_walk(cx);
        }
        // A low-level settled route may be frozen `'closest'`
        // (`WalkExecutor.ts:316–325`), but frozen resilient arrival is only
        // `isArrived` (`Traversal.ts:118–142`). With no progress,
        // `walkLadder.ts:73–77` advances baked → scene.
        self.kick_scene(cx)
    }

    fn after_scene(&mut self, cx: &mut Cx<'_>) -> Option<bool> {
        let Some(here) = here() else {
            return Some(false);
        };
        let cur = walk_chebyshev(here, self.dest);
        if cur < self.best_dist {
            self.best_dist = cur;
            self.no_progress = 0;
            return self.kick_walk(cx);
        }

        // Frozen follows scene with a door / nearby-step unstick
        // (`Traversal.ts:179–189`, `walkLadder.ts:79–86`).
        self.kick_unstick(cx)
    }

    fn kick_unstick(&mut self, cx: &mut Cx<'_>) -> Option<bool> {
        let Some(me) = here() else {
            return Some(false);
        };
        if let Some(door) = pick_helpful_door(me, self.dest, self.radius, unstick_candidates(me)) {
            let Some(op) = op_starting(&door, "open") else {
                return self.kick_unstick_step(cx);
            };
            self.logs.push_back(format!(
                "stalled next to closed '{}' at ({},{}) — opening it",
                door.name_or_empty(),
                door.x,
                door.z
            ));
            emit_loc_open(&door, &op, cx);
            cx.clock().arm(UNSTICK_DOOR_MS);
            self.phase = Phase::UnstickDoor {
                tile: loc_world(&door),
                name: door.name_or_empty().to_string(),
            };
            return None;
        }
        self.kick_unstick_step(cx)
    }

    fn kick_unstick_step(&mut self, cx: &mut Cx<'_>) -> Option<bool> {
        let Some(me) = here() else {
            return self.after_no_progress_pass();
        };
        let step = pick_unstick_step(me, self.unstick_dir);
        self.unstick_dir = (self.unstick_dir + 3) % 8;
        if let Some((dx, dz)) = step {
            let target = WorldTile {
                x: me.x + dx,
                z: me.z + dz,
                level: me.level,
            };
            cx.emit(InteractReq::WalkTo {
                x: target.x,
                z: target.z,
                level: target.level,
            });
            let last_issued = scene_now(cx);
            cx.clock().arm(UNSTICK_STEP_MS);
            self.phase = Phase::UnstickStep(UnstickStep {
                target,
                last_issued,
                last_tile: me,
                delay_left: SCENE_CHECK_TICKS,
            });
            return None;
        }
        self.after_unstick_step(cx)
    }

    fn after_unstick_step(&mut self, cx: &mut Cx<'_>) -> Option<bool> {
        let Some(current) = here() else {
            return Some(false);
        };
        let distance = walk_chebyshev(current, self.dest);
        if distance < self.best_dist {
            self.best_dist = distance;
            self.no_progress = 0;
            self.kick_walk(cx)
        } else {
            self.after_no_progress_pass()
        }
    }

    fn after_displaced_walk(&mut self) -> Option<bool> {
        // Another native walk replaced this wait before its deadline. The
        // timed-out Walk emitted only its fenced AbortWalk; skip the unfenced
        // scene click for this pass. The legacy pass/backoff behavior below
        // may later re-arm, exactly as it did before this scene phase existed.
        self.after_no_progress_pass()
    }

    fn after_no_progress_pass(&mut self) -> Option<bool> {
        self.no_progress += 1;
        if let Some(max) = self.attempts {
            if self.no_progress >= max {
                self.logs.push_back(format!(
                    "walkResilient: {max} passes made no progress — stopping (bounded caller)"
                ));
                return Some(false);
            }
        }
        if self.no_progress >= UNREACHABLE_PASSES {
            // Frozen verify then probe-dead → unreachable (walkLadder.ts:66–68,
            // 83–84). No probeDest: fail-closed as dead.
            self.logs.push_back(format!(
                "walkResilient: ({},{},{}) unreachable from here — stopping (best {} tiles)",
                self.dest.x, self.dest.z, self.dest.level, self.best_dist
            ));
            return Some(false);
        }
        self.delay_left = backoff_ticks(self.no_progress);
        self.phase = Phase::Backoff;
        None
    }
}

/// Frozen `Traversal.walkResilient(dest, opts)`.
pub(crate) struct WalkResilient {
    drive: Resilient,
    pumped: bool,
    result: Option<bool>,
    waiting: bool,
}

impl Family for WalkResilient {
    const NAME: &'static str = "walk-resilient";
    const CALLBACKS: &'static [&'static str] = &["log", "sustain"];
    /// Frozen `log(...)` is not awaited; `await Sustain.run()` is.
    const SYNC_HOOKS: &'static [usize] = &[LOG];
    /// The first baked walk goes out in the caller's turn.
    const KICK_ON_START: bool = true;
    type Args = WalkResilientArgs;
    type Output = bool;

    fn begin(args: WalkResilientArgs, _cx: &mut Cx<'_>) -> Begin<Self> {
        if interrupted() {
            return Begin::Done(false);
        }
        let dest = args.tile.world();
        let radius = args.opts.radius;
        let Some(_) = here() else {
            return Begin::Done(false);
        };
        if arrived(dest, radius) {
            return Begin::Done(true);
        }
        Begin::Run(Self {
            drive: Resilient::new(
                dest,
                radius,
                args.opts.timeout_ms.unwrap_or(BAKED_TIMEOUT_MS),
                args.opts.attempts,
                args.opts.use_teleport_catalog,
            ),
            pumped: false,
            result: None,
            waiting: false,
        })
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<bool> {
        if let Some(Reply::Threw(thrown)) = cx.reply() {
            return Step::Fail(thrown);
        }
        loop {
            if let Some(line) = self.drive.pop_log() {
                if cx.has(LOG) {
                    return Step::Call(Call {
                        hook: LOG,
                        args: vec![json!(line)],
                    });
                }
                continue;
            }
            if let Some(result) = self.result {
                return Step::Done(result);
            }
            if std::mem::take(&mut self.waiting) {
                self.pumped = false;
                return Step::Wait;
            }
            // Frozen `await Sustain.run()` (`Traversal.ts:130`); hunt-wait-fed
            // pumps the same hook so cards that eat during walks keep eating.
            if !self.pumped && cx.has(SUSTAIN) {
                self.pumped = true;
                return Step::Call(Call {
                    hook: SUSTAIN,
                    args: Vec::new(),
                });
            }
            match self.drive.step(cx) {
                Some(result) => self.result = Some(result),
                None if self.drive.logs.is_empty() => {
                    self.pumped = false;
                    return Step::Wait;
                }
                None => self.waiting = true,
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WalkOpeningArgs {
    dest: Tile,
    #[serde(default)]
    radius: i32,
    #[serde(default)]
    obstacles: Vec<String>,
}

enum OpeningPhase {
    NeedWalk,
    Walking(Walk),
    Approach { door: WorldTile, walk: Walk },
    Open { door: WorldTile },
    Wait { door: WorldTile },
}

/// Frozen `walkOpening` (`walkOpening.ts:101–161`): eight baked segments,
/// then an openable obstacle within 14. Frozen prefers toward-dest
/// (`walkOpening.ts:127–136`); operator refinement uses the same
/// path-relevant counterfactual as unstick (toward-dest is weaker).
/// `doorStepTicks` / `stepThrough` is not ported; the next segment walks
/// the opened passage (frozen's default when that opt is omitted).
pub(crate) struct WalkOpening {
    dest: WorldTile,
    radius: i32,
    obstacles: Vec<String>,
    segment: u32,
    phase: OpeningPhase,
    logs: VecDeque<String>,
    result: Option<bool>,
    waiting: bool,
}

impl WalkOpening {
    fn abort_walk(&self, cx: &mut Cx<'_>) {
        match &self.phase {
            OpeningPhase::Walking(walk) | OpeningPhase::Approach { walk, .. } => walk.abort(cx),
            _ => {}
        }
    }

    fn pop_log(&mut self) -> Option<String> {
        self.logs.pop_front()
    }

    fn advance(&mut self, cx: &mut Cx<'_>) -> Option<bool> {
        if interrupted() {
            self.abort_walk(cx);
            self.logs
                .push_back("walk interrupted by a runtime event — yielding to the runtime".into());
            return Some(false);
        }
        if arrived(self.dest, self.radius) {
            return Some(true);
        }
        loop {
            match std::mem::replace(&mut self.phase, OpeningPhase::NeedWalk) {
                OpeningPhase::NeedWalk => {
                    if self.segment >= OPENING_SEGMENTS {
                        return Some(arrived(self.dest, self.radius));
                    }
                    match Walk::begin(self.dest, self.radius, BAKED_TIMEOUT_MS, false, cx) {
                        Ok(walk) => {
                            self.phase = OpeningPhase::Walking(walk);
                            return None;
                        }
                        Err(done) => return Some(done),
                    }
                }
                OpeningPhase::Walking(walk) => match walk.step(cx) {
                    None => {
                        self.phase = OpeningPhase::Walking(walk);
                        return None;
                    }
                    Some(_) => {
                        if arrived(self.dest, self.radius) {
                            return Some(true);
                        }
                        let Some(me) = here() else {
                            return Some(false);
                        };
                        let Some(door) =
                            pick_opening_door(me, self.dest, self.radius, &self.obstacles)
                        else {
                            return Some(false);
                        };
                        let tile = loc_world(&door);
                        if walk_chebyshev(me, tile) > 1 {
                            self.logs.push_back(format!(
                                "walking to {} at {},{} to open it",
                                door.name_or_empty(),
                                tile.x,
                                tile.z
                            ));
                            match Walk::begin(tile, 1, OPENING_DOOR_WALK_MS, false, cx) {
                                Ok(walk) => {
                                    self.phase = OpeningPhase::Approach { door: tile, walk };
                                    return None;
                                }
                                Err(true) => {
                                    self.phase = OpeningPhase::Open { door: tile };
                                }
                                Err(false) => return Some(false),
                            }
                        } else {
                            self.phase = OpeningPhase::Open { door: tile };
                        }
                    }
                },
                OpeningPhase::Approach { door, walk } => match walk.step(cx) {
                    None => {
                        self.phase = OpeningPhase::Approach { door, walk };
                        return None;
                    }
                    Some(_) => self.phase = OpeningPhase::Open { door },
                },
                OpeningPhase::Open { door } => {
                    let Some(shut) = shut_obstacle_at(door, &self.obstacles) else {
                        self.segment += 1;
                        continue;
                    };
                    let Some(op) = op_starting(&shut, "open") else {
                        self.segment += 1;
                        continue;
                    };
                    self.logs.push_back(format!(
                        "opening {} at {},{}",
                        shut.name_or_empty(),
                        door.x,
                        door.z
                    ));
                    emit_loc_open(&shut, &op, cx);
                    cx.clock().arm(OPEN_WAIT_MS);
                    self.phase = OpeningPhase::Wait { door };
                    return None;
                }
                OpeningPhase::Wait { door } => {
                    if shut_obstacle_at(door, &self.obstacles).is_none()
                        || cx.clock().bound_reached()
                    {
                        self.segment += 1;
                        continue;
                    }
                    self.phase = OpeningPhase::Wait { door };
                    return None;
                }
            }
        }
    }
}

impl Family for WalkOpening {
    const NAME: &'static str = "walk-opening";
    const CALLBACKS: &'static [&'static str] = &["log"];
    const SYNC_HOOKS: &'static [usize] = &[LOG];
    const KICK_ON_START: bool = true;
    type Args = WalkOpeningArgs;
    type Output = bool;

    fn begin(args: WalkOpeningArgs, _cx: &mut Cx<'_>) -> Begin<Self> {
        if interrupted() {
            return Begin::Done(false);
        }
        let dest = args.dest.world();
        let Some(_) = here() else {
            return Begin::Done(false);
        };
        if arrived(dest, args.radius) {
            return Begin::Done(true);
        }
        Begin::Run(Self {
            dest,
            radius: args.radius,
            obstacles: args.obstacles,
            segment: 0,
            phase: OpeningPhase::NeedWalk,
            logs: VecDeque::new(),
            result: None,
            waiting: false,
        })
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<bool> {
        if let Some(Reply::Threw(thrown)) = cx.reply() {
            return Step::Fail(thrown);
        }
        loop {
            if let Some(line) = self.pop_log() {
                if cx.has(LOG) {
                    return Step::Call(Call {
                        hook: LOG,
                        args: vec![json!(line)],
                    });
                }
                continue;
            }
            if let Some(result) = self.result {
                return Step::Done(result);
            }
            if std::mem::take(&mut self.waiting) {
                return Step::Wait;
            }
            match self.advance(cx) {
                Some(result) => self.result = Some(result),
                None if self.logs.is_empty() => return Step::Wait,
                None => self.waiting = true,
            }
        }
    }
}

const LOG: usize = 0;
const SUSTAIN: usize = 1;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load::callback_v8::HeldCallback;
    use crate::machine::{self, Called, Outcome, Pending, Started, Take};
    use crate::observed::{self, WalkOutcome};
    use crate::walk_wait;
    use serde_json::Value;

    struct NoJs;

    impl machine::Js for NoJs {
        fn queue_len(&mut self) -> usize {
            0
        }

        fn call(&mut self, _hook: Option<&HeldCallback>, _args: &[Value]) -> Called {
            panic!("no hook is held");
        }

        fn poll(&mut self, _pending: &Pending) -> Option<Reply> {
            panic!("no hook is held");
        }

        fn claimed(&mut self) -> bool {
            false
        }
    }

    fn reset() {
        observed::on_reset();
        machine::on_reset();
        walk_wait::on_reset();
        crate::load::reach_query::on_reset();
        machine::on_hold(false);
        reset_scene_time();
    }

    fn post_here(x: i32, z: i32) {
        observed::post(1, |post| {
            post.session(true).here(observed::Tile { x, z, level: 0 });
        });
    }

    fn start(attempts: Option<u32>) -> machine::Handle {
        let mut args = json!({ "tile": { "x": 10, "z": 0, "level": 0 }, "opts": { "radius": 0 } });
        if let Some(attempts) = attempts {
            args["opts"]["attempts"] = json!(attempts);
        }
        let Started::Running(h) = machine::start("walk-resilient", args, Vec::new(), 0) else {
            panic!("walk-resilient runs");
        };
        h
    }

    fn walk_token() -> u64 {
        match machine::merge_ops(Vec::new()).as_slice() {
            [InteractReq::Walk {
                x: 10,
                z: 0,
                request_id,
                ..
            }]
            | [InteractReq::WalkNear {
                x: 10,
                z: 0,
                request_id,
                ..
            }] => *request_id,
            other => panic!("expected the walk, got {other:?}"),
        }
    }

    fn post_walk_outcome(
        seq: u64,
        token: u64,
        here: WorldTile,
        dest: WorldTile,
        radius: i32,
        failed: bool,
    ) {
        observed::post(seq, |post| {
            post.session(true)
                .here(observed::Tile {
                    x: here.x,
                    z: here.z,
                    level: here.level,
                })
                .walk_outcome(WalkOutcome {
                    seq,
                    generation: 0,
                    request_id: token,
                    failed,
                    tile: observed::Tile {
                        x: dest.x,
                        z: dest.z,
                        level: dest.level,
                    },
                    radius,
                    allow_teleports: false,
                });
        });
    }

    fn fail_walk(seq: u64, token: u64, x: i32, z: i32, failed: bool) {
        post_walk_outcome(
            seq,
            token,
            WorldTile { x, z, level: 0 },
            WorldTile {
                x: 10,
                z: 0,
                level: 0,
            },
            0,
            failed,
        );
    }

    #[test]
    fn already_arrived_is_done_without_a_walk() {
        reset();
        post_here(10, 0);
        let args = json!({ "tile": { "x": 10, "z": 0, "level": 0 }, "opts": { "radius": 0 } });
        match machine::start("walk-resilient", args, Vec::new(), 0) {
            Started::Settled(Outcome::Done(Value::Bool(true))) => {}
            other => panic!("arrived begin is done true, got {other:?}"),
        }
        assert!(machine::merge_ops(Vec::new()).is_empty());
    }

    #[test]
    fn interrupt_at_start_is_done_false() {
        reset();
        observed::post(1, |post| {
            post.session(true)
                .here(observed::Tile {
                    x: 0,
                    z: 0,
                    level: 0,
                })
                .ours(true);
        });
        let args = json!({ "tile": { "x": 10, "z": 0, "level": 0 }, "opts": { "radius": 0 } });
        match machine::start("walk-resilient", args, Vec::new(), 0) {
            Started::Settled(Outcome::Done(Value::Bool(false))) => {}
            other => panic!("pending EventSignal is done false, got {other:?}"),
        }
    }

    #[test]
    fn nopath_without_attempts_backoffs_instead_of_ending() {
        reset();
        post_here(0, 0);
        let h = start(None);
        machine::step(&mut NoJs);
        let token = walk_token();
        fail_walk(1, token, 0, 0, true);
        machine::step(&mut NoJs);
        assert_eq!(
            machine::merge_ops(Vec::new()),
            vec![InteractReq::WalkTo {
                x: 10,
                z: 0,
                level: 0,
            }],
            "no-progress baked failure advances to the frozen scene step"
        );
        machine::step(&mut NoJs);
        assert!(
            machine::merge_ops(Vec::new()).is_empty(),
            "DirectNavigator waits two ticks between scene checks"
        );
        machine::step(&mut NoJs);
        assert_eq!(
            machine::merge_ops(Vec::new()),
            vec![InteractReq::WalkTo {
                x: 10,
                z: 0,
                level: 0,
            }],
            "a stalled DirectNavigator reissues its nearest scene click"
        );
        machine::age(h, SCENE_TIMEOUT_MS + 1);
        machine::step(&mut NoJs);
        assert!(
            machine::merge_ops(Vec::new()).is_empty(),
            "the completed scene pass backoffs instead of ending"
        );
        assert_eq!(machine::take(h), Take::Pending);
    }

    #[test]
    fn scene_walk_emits_the_clamped_target_on_the_current_plane() {
        reset();
        post_here(0, 0);
        let dest = WorldTile {
            x: 200,
            z: 60,
            level: 1,
        };
        let args = json!({
            "tile": { "x": dest.x, "z": dest.z, "level": dest.level },
            "opts": { "radius": 0 },
        });
        let Started::Running(h) = machine::start("walk-resilient", args, Vec::new(), 0) else {
            panic!("walk-resilient runs");
        };
        machine::step(&mut NoJs);
        let token = match machine::merge_ops(Vec::new()).as_slice() {
            [InteractReq::Walk {
                x: 200,
                z: 60,
                level: 1,
                request_id,
                ..
            }] => *request_id,
            other => panic!("expected the baked walk, got {other:?}"),
        };
        post_walk_outcome(
            2,
            token,
            WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            dest,
            0,
            true,
        );
        machine::step(&mut NoJs);
        assert_eq!(
            machine::merge_ops(Vec::new()),
            vec![InteractReq::WalkTo {
                x: 48,
                z: 48,
                level: 0,
            }]
        );
        assert_eq!(machine::take(h), Take::Pending);
    }

    #[test]
    fn moving_scene_step_reclicks_at_2400_ms_not_before() {
        reset();
        post_here(0, 0);
        let h = start(None);
        machine::step(&mut NoJs);
        let token = walk_token();
        fail_walk(1, token, 0, 0, true);
        machine::step(&mut NoJs);
        assert!(matches!(
            machine::merge_ops(Vec::new()).as_slice(),
            [InteractReq::WalkTo { .. }]
        ));

        age_scene_time(SCENE_RECLICK_MS - 1);
        observed::post(2, |post| {
            post.session(true).here(observed::Tile {
                x: 1,
                z: 0,
                level: 0,
            });
        });
        machine::step(&mut NoJs);
        machine::step(&mut NoJs);
        assert!(
            machine::merge_ops(Vec::new()).is_empty(),
            "moving before 2400 ms must not re-click"
        );

        age_scene_time(SCENE_RECLICK_MS);
        observed::post(3, |post| {
            post.session(true).here(observed::Tile {
                x: 2,
                z: 0,
                level: 0,
            });
        });
        machine::step(&mut NoJs);
        machine::step(&mut NoJs);
        assert_eq!(
            machine::merge_ops(Vec::new()),
            vec![InteractReq::WalkTo {
                x: 10,
                z: 0,
                level: 0,
            }]
        );
        assert_eq!(machine::take(h), Take::Pending);
    }

    #[test]
    fn scene_radius_plus_one_progress_restarts_the_baked_walk() {
        reset();
        post_here(0, 0);
        let h = start(None);
        machine::step(&mut NoJs);
        let token = walk_token();
        fail_walk(1, token, 0, 0, true);
        machine::step(&mut NoJs);
        assert!(matches!(
            machine::merge_ops(Vec::new()).as_slice(),
            [InteractReq::WalkTo { .. }]
        ));

        observed::post(2, |post| {
            post.session(true).here(observed::Tile {
                x: 9,
                z: 0,
                level: 0,
            });
        });
        machine::step(&mut NoJs);
        let retry = walk_token();
        assert_ne!(retry, token);
        assert_eq!(machine::take(h), Take::Pending);
    }

    #[test]
    fn three_no_progress_passes_stop_as_unreachable() {
        reset();
        post_here(0, 0);
        let h = start(None);
        for (seq, pass) in (1..).zip(0..UNREACHABLE_PASSES) {
            machine::step(&mut NoJs);
            let token = walk_token();
            fail_walk(seq, token, 0, 0, true);
            machine::step(&mut NoJs);
            assert_eq!(
                machine::merge_ops(Vec::new()),
                vec![InteractReq::WalkTo {
                    x: 10,
                    z: 0,
                    level: 0,
                }]
            );
            machine::age(h, SCENE_TIMEOUT_MS + 1);
            machine::step(&mut NoJs);
            if pass + 1 < UNREACHABLE_PASSES {
                let ticks = backoff_ticks(pass + 1);
                for _ in 0..ticks {
                    machine::step(&mut NoJs);
                }
            }
        }
        assert_eq!(machine::take(h), Take::Settled(Outcome::Done(json!(false))));
    }

    #[test]
    fn closer_tile_rebakes_after_nopath() {
        reset();
        post_here(0, 0);
        let h = start(Some(3));
        machine::step(&mut NoJs);
        let token = walk_token();
        fail_walk(1, token, 5, 0, true);
        machine::step(&mut NoJs);
        let retry = walk_token();
        assert_ne!(retry, 0);
        assert_ne!(retry, token);
        assert_eq!(machine::take(h), Take::Pending);
    }

    #[test]
    fn closest_terminal_without_arrival_is_not_success() {
        reset();
        post_here(0, 0);
        let h = start(None);
        machine::step(&mut NoJs);
        let token = walk_token();
        fail_walk(1, token, 0, 0, false);
        machine::step(&mut NoJs);
        assert_eq!(
            machine::take(h),
            Take::Pending,
            "N1: isArrived, not wait true"
        );
        assert_eq!(
            machine::merge_ops(Vec::new()),
            vec![InteractReq::WalkTo {
                x: 10,
                z: 0,
                level: 0,
            }]
        );
    }

    fn blocked_booth_across_long_wall(
    ) -> (api::snapshot::SceneView, WorldTile, WorldTile, WorldTile) {
        use client::dash3d::CollisionFlag;

        let mut scene = api::snapshot::SceneView {
            available: true,
            base_x: 0,
            base_z: 0,
            level: 0,
            width: 64,
            height: 64,
            collision_flags: vec![0; 64 * 64],
        };
        for z in 8..=55 {
            scene.collision_flags[(31 * 64 + z) as usize] |= CollisionFlag::W_E;
            scene.collision_flags[(32 * 64 + z) as usize] |= CollisionFlag::W_W;
        }
        let dest = WorldTile {
            x: 34,
            z: 32,
            level: 0,
        };
        scene.collision_flags[(dest.x * 64 + dest.z) as usize] |= CollisionFlag::SQ_BLOCKED;
        (
            scene,
            dest,
            WorldTile {
                x: 31,
                z: 32,
                level: 0,
            },
            WorldTile {
                x: 33,
                z: 32,
                level: 0,
            },
        )
    }

    #[test]
    fn walk_resilient_closest_with_fresh_negative_reach_does_not_arrive() {
        use crate::isolate_fb::{
            encode_snapshot_with_native, NativeFactsInput, ReachViewInput, SnapshotReader,
            TileInput,
        };

        reset();
        post_here(30, 28);
        let (scene, dest, outside, inside) = blocked_booth_across_long_wall();
        let args = json!({
            "tile": { "x": dest.x, "z": dest.z, "level": dest.level },
            "opts": { "radius": 3 },
        });
        let Started::Running(h) = machine::start("walk-resilient", args, Vec::new(), 0) else {
            panic!("walk-resilient runs");
        };
        let take_walk_token = || match machine::merge_ops(Vec::new()).as_slice() {
            [InteractReq::WalkNear {
                x,
                z,
                level,
                radius: 3,
                request_id,
                ..
            }] if (*x, *z, *level) == (dest.x, dest.z, dest.level) => *request_id,
            other => panic!("expected the fixture walk, got {other:?}"),
        };
        machine::step(&mut NoJs);
        let first_token = take_walk_token();

        let post = |post_number: u64,
                    here: WorldTile,
                    outcome: Option<(u64, u64)>|
         -> api::query::ReachQueryView {
            let flood = api::query::SceneQuery::new(&scene, Some(here))
                .flood_reach()
                .expect("fixture origin is in-scene");
            let view = api::query::pack_reach_query(&scene, Some(&flood));
            let here_index =
                ((here.x - scene.base_x) * scene.height + here.z - scene.base_z) as usize;
            assert_eq!(
                view.exact_rank[here_index], 0,
                "the posted reach must be fresh for here"
            );

            let mut input = crate::isolate_fb::tests::empty_input(post_number);
            input.here = Some(TileInput {
                x: here.x,
                z: here.z,
                level: here.level,
            });
            input.reach = ReachViewInput {
                available: view.available,
                base_x: view.base_x,
                base_z: view.base_z,
                level: view.level,
                width: view.width,
                height: view.height,
                walkable: &view.walkable,
                reachable: &view.reachable,
                reachable_adj: &view.reachable_adj,
                exact_rank: &view.exact_rank,
                adjacent_rank: &view.adjacent_rank,
                step: &view.step,
                canlight: &view.canlight,
                stamp: post_number,
            };
            let native =
                outcome.map_or_else(NativeFactsInput::default, |(seq, token)| NativeFactsInput {
                    walk_outcome_seq: seq,
                    walk_outcome_request_id: token,
                    walk_outcome_failed: false,
                    walk_outcome_x: dest.x,
                    walk_outcome_z: dest.z,
                    walk_outcome_level: dest.level,
                    walk_outcome_radius: 3,
                    walk_outcome_allow_teleports: false,
                    ..Default::default()
                });
            let bytes = encode_snapshot_with_native(&input, native);
            let snap = SnapshotReader::from_bytes(&bytes).expect("snapshot");
            observed::apply(&snap);
            crate::load::reach_query::apply(&snap);
            walk_wait::on_snapshot(&snap);
            view
        };

        let outside_view = post(2, outside, Some((1, first_token)));
        assert!(
            !api::query::is_arrived(outside, dest, 3, || &outside_view),
            "the fresh bounded probe cannot reach around the long wall"
        );
        machine::step(&mut NoJs);
        assert_eq!(
            machine::take(h),
            Take::Pending,
            "a matched closest route end settles the low-level wait, not resilient arrival"
        );

        let retry_token = take_walk_token();
        post(3, outside, Some((2, retry_token)));
        machine::step(&mut NoJs);
        assert_eq!(
            machine::merge_ops(Vec::new()),
            vec![InteractReq::WalkTo {
                x: dest.x,
                z: dest.z,
                level: dest.level,
            }],
            "a no-progress baked closest follows the frozen ladder's scene step"
        );
        assert_eq!(machine::take(h), Take::Pending);

        let inside_view = post(4, inside, None);
        assert!(
            api::query::is_arrived(inside, dest, 3, || &inside_view),
            "the destination-side stand is arrival-capable"
        );
        machine::step(&mut NoJs);
        assert_eq!(machine::take(h), Take::Settled(Outcome::Done(json!(true))));
    }

    #[test]
    fn ours_after_nopath_returns_false() {
        reset();
        post_here(0, 0);
        let h = start(None);
        machine::step(&mut NoJs);
        let token = walk_token();
        observed::post(2, |post| {
            post.session(true)
                .here(observed::Tile {
                    x: 0,
                    z: 0,
                    level: 0,
                })
                .ours(true)
                .walk_outcome(WalkOutcome {
                    seq: 1,
                    generation: 0,
                    request_id: token,
                    failed: true,
                    tile: observed::Tile {
                        x: 10,
                        z: 0,
                        level: 0,
                    },
                    radius: 0,
                    allow_teleports: false,
                });
        });
        machine::step(&mut NoJs);
        assert_eq!(machine::take(h), Take::Settled(Outcome::Done(json!(false))));
    }

    #[test]
    fn guardian_hold_pauses_the_row() {
        reset();
        post_here(0, 0);
        let h = start(None);
        machine::step(&mut NoJs);
        let _token = walk_token();
        machine::on_hold(true);
        machine::step(&mut NoJs);
        assert!(machine::merge_ops(Vec::new()).is_empty());
        assert_eq!(machine::take(h), Take::Pending);
        machine::on_hold(false);
    }

    #[test]
    fn displaced_walk_skips_the_unowned_scene_click() {
        reset();
        post_here(0, 0);
        let h = start(None);
        machine::step(&mut NoJs);
        let displaced = walk_token();
        let replacement = walk_wait::dispatch(&json!({
            "op": "begin",
            "x": 20,
            "z": 0,
            "level": 0,
            "radius": 0,
            "allow_teleports": false,
        }))
        .as_u64()
        .expect("replacement token");
        assert!(walk_wait::owns(replacement));
        assert!(!walk_wait::owns(displaced));

        machine::tests::expire_deadlines();
        machine::step(&mut NoJs);
        assert_eq!(
            machine::merge_ops(Vec::new()),
            vec![InteractReq::AbortWalk {
                request_id: displaced,
            }],
            "a displaced wait may abort its old host follow but not issue an unfenced scene walk"
        );
        assert_eq!(machine::take(h), Take::Pending);
    }

    #[test]
    fn a_timed_out_walk_stops_the_host_follow() {
        reset();
        post_here(0, 0);
        let h = start(None);
        machine::step(&mut NoJs);
        let walk_token = walk_token();
        assert_ne!(walk_token, 0);
        machine::step(&mut NoJs);
        assert!(machine::merge_ops(Vec::new()).is_empty(), "still walking");
        machine::tests::expire_deadlines();
        machine::step(&mut NoJs);
        assert_eq!(
            machine::merge_ops(Vec::new()),
            vec![
                InteractReq::AbortWalk {
                    request_id: walk_token
                },
                InteractReq::WalkTo {
                    x: 10,
                    z: 0,
                    level: 0,
                },
            ],
            "the timed-out baked route is stopped before the scene ladder step",
        );
        assert_eq!(machine::take(h), Take::Pending);
    }

    fn door_loc(id: i32, x: i32, z: i32) -> crate::observed::SceneRow {
        let dist = x.abs().max(z.abs());
        crate::observed::SceneRow {
            id,
            name: Some("Door".into()),
            x,
            z,
            level: 0,
            distance: dist,
            actions: vec!["Open".into(), "Examine".into()].into(),
            shape: client::dash3d::LocShape::CENTREPIECE_STRAIGHT as u8,
            angle: client::dash3d::LocAngle::WEST as u8,
        }
    }

    #[test]
    fn op_prefix_on_non_ascii_action_fails_closed() {
        let mut loc = door_loc(1530, 0, 0);
        loc.actions = vec!["Öffnen".into()].into();
        assert_eq!(op_starting(&loc, "o"), None);
    }

    fn counterfactual_scene(fill: i32) -> api::snapshot::SceneView {
        api::snapshot::SceneView {
            available: true,
            base_x: -16,
            base_z: -16,
            level: 0,
            width: 32,
            height: 32,
            collision_flags: vec![fill; 32 * 32],
        }
    }

    fn counterfactual_or(scene: &mut api::snapshot::SceneView, x: i32, z: i32, bits: i32) {
        let i = ((x - scene.base_x) * scene.height + z - scene.base_z) as usize;
        scene.collision_flags[i] |= bits;
    }

    fn counterfactual_set(scene: &mut api::snapshot::SceneView, x: i32, z: i32, bits: i32) {
        let i = ((x - scene.base_x) * scene.height + z - scene.base_z) as usize;
        scene.collision_flags[i] = bits;
    }

    fn post_counterfactual_scene(
        here: WorldTile,
        scene: &api::snapshot::SceneView,
        doors: Vec<crate::observed::SceneRow>,
    ) {
        use std::sync::Arc;
        observed::post(1, |post| {
            post.session(true)
                .here(observed::Tile {
                    x: here.x,
                    z: here.z,
                    level: here.level,
                })
                .locs(doors)
                .collision(api::line_of_sight::CollisionQuery {
                    available: true,
                    base_x: scene.base_x,
                    base_z: scene.base_z,
                    level: scene.level,
                    width: scene.width,
                    height: scene.height,
                    flags: Arc::from(scene.collision_flags.as_slice()),
                });
        });
    }

    fn straight_wall_door(door_on_player_side: bool) -> Option<crate::observed::SceneRow> {
        use client::dash3d::CollisionFlag;
        let mut scene = counterfactual_scene(0);
        for z in -16..16 {
            counterfactual_or(&mut scene, 1, z, CollisionFlag::W_E);
            counterfactual_or(&mut scene, 2, z, CollisionFlag::W_W);
        }
        let mut door = if door_on_player_side {
            door_loc(1530, 1, 0)
        } else {
            door_loc(1530, 2, 0)
        };
        door.shape = client::dash3d::LocShape::WALL_STRAIGHT as u8;
        door.angle = if door_on_player_side {
            client::dash3d::LocAngle::EAST
        } else {
            client::dash3d::LocAngle::WEST
        } as u8;
        let me = WorldTile {
            x: 0,
            z: 0,
            level: 0,
        };
        post_counterfactual_scene(me, &scene, vec![door.clone()]);
        pick_helpful_door(
            me,
            WorldTile {
                x: 10,
                z: 0,
                level: 0,
            },
            0,
            vec![door],
        )
    }

    #[test]
    fn straight_wall_door_is_helpful_from_either_placement_side() {
        reset();
        assert!(
            straight_wall_door(false).is_some(),
            "control: clearing the far-side placement opens the crossing"
        );
        reset();
        assert!(
            straight_wall_door(true).is_some(),
            "the reciprocal edge must also be cleared for a player-side placement"
        );
    }

    #[test]
    fn helpful_door_tie_break_uses_shortest_route_depth() {
        use client::dash3d::CollisionFlag;
        reset();
        let mut scene = counterfactual_scene(CollisionFlag::SQ_BLOCKED);
        for x in -1..=1 {
            for z in -1..=1 {
                counterfactual_set(&mut scene, x, z, 0);
            }
        }
        for x in 3..=15 {
            for z in -15..=3 {
                counterfactual_set(&mut scene, x, z, 0);
            }
        }
        for z in 3..=8 {
            counterfactual_set(&mut scene, 0, z, 0);
        }
        for x in 0..=12 {
            counterfactual_set(&mut scene, x, 8, 0);
        }
        for z in 4..=8 {
            counterfactual_set(&mut scene, 12, z, 0);
        }
        let me = WorldTile {
            x: 0,
            z: 0,
            level: 0,
        };
        let short = door_loc(1530, 2, 0);
        let long = door_loc(1531, 0, 2);
        post_counterfactual_scene(me, &scene, vec![short.clone(), long.clone()]);
        let picked = pick_helpful_door(
            me,
            WorldTile {
                x: 10,
                z: 0,
                level: 0,
            },
            0,
            vec![long, short],
        )
        .expect("both openings make the destination routable");
        assert_eq!(
            (picked.x, picked.z),
            (2, 0),
            "the depth-10 route must beat the depth-28 route"
        );
    }

    #[test]
    fn unstick_excludes_desert_mining_camp_scripted_doors() {
        reset();
        post_counterfactual_scene(
            WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            &counterfactual_scene(0),
            vec![door_loc(2673, 1, 0), door_loc(1530, 2, 0)],
        );
        let candidates = unstick_candidates(WorldTile {
            x: 0,
            z: 0,
            level: 0,
        });
        assert_eq!(
            candidates.iter().map(|loc| loc.id).collect::<Vec<_>>(),
            vec![1530]
        );
    }

    fn wall_west_of_dest() -> api::snapshot::SceneView {
        use client::dash3d::CollisionFlag;
        let mut scene = api::snapshot::SceneView {
            available: true,
            base_x: -16,
            base_z: -16,
            level: 0,
            width: 32,
            height: 32,
            collision_flags: vec![0; 32 * 32],
        };
        for z in -8..=8 {
            let lx = 2 - scene.base_x;
            let lz = z - scene.base_z;
            scene.collision_flags[(lx * scene.height + lz) as usize] |= CollisionFlag::SQ_BLOCKED;
        }
        scene
    }

    fn post_walled_scene(doors: Vec<crate::observed::SceneRow>) {
        use std::sync::Arc;
        let scene = wall_west_of_dest();
        let here = WorldTile {
            x: 0,
            z: 0,
            level: 0,
        };
        observed::post(1, |post| {
            post.session(true)
                .here(observed::Tile {
                    x: 0,
                    z: 0,
                    level: 0,
                })
                .locs(doors)
                .collision(api::line_of_sight::CollisionQuery {
                    available: true,
                    base_x: scene.base_x,
                    base_z: scene.base_z,
                    level: scene.level,
                    width: scene.width,
                    height: scene.height,
                    flags: Arc::from(scene.collision_flags.as_slice()),
                });
        });
        let flood = api::query::SceneQuery::new(&scene, Some(here))
            .flood_reach()
            .expect("origin is in-scene");
        crate::load::reach_query::set_view_for_tests(api::query::pack_reach_query(
            &scene,
            Some(&flood),
        ));
    }

    fn fail_baked_then_scene(h: machine::Handle) {
        machine::step(&mut NoJs);
        let token = walk_token();
        fail_walk(1, token, 0, 0, true);
        machine::step(&mut NoJs);
        assert_eq!(
            machine::merge_ops(Vec::new()),
            vec![InteractReq::WalkTo {
                x: 10,
                z: 0,
                level: 0,
            }],
            "no-progress baked failure advances to the frozen scene step"
        );
        machine::age(h, SCENE_TIMEOUT_MS + 1);
        machine::step(&mut NoJs);
    }

    #[test]
    fn unstick_opens_a_nearby_door_then_takes_one_scene_step() {
        reset();
        post_walled_scene(vec![door_loc(1530, 2, 0)]);
        let h = start(None);
        fail_baked_then_scene(h);
        assert_eq!(
            machine::merge_ops(Vec::new()),
            vec![InteractReq::Loc {
                x: 2,
                z: 0,
                level: 0,
                action: "Open".into(),
                id: Some(1530),
            }],
            "a door whose opening makes dest routable is opened"
        );
        observed::post(3, |post| {
            post.locs(Vec::new());
        });
        machine::step(&mut NoJs);
        assert_eq!(
            machine::merge_ops(Vec::new()),
            vec![InteractReq::WalkTo {
                x: 0,
                z: 1,
                level: 0,
            }],
            "pickUnstickStep then one scene WalkTo before any unreachable log"
        );
        assert_eq!(machine::take(h), Take::Pending);
    }

    #[test]
    fn progress_during_unstick_step_restarts_baked_without_counting_pass() {
        reset();
        post_walled_scene(vec![door_loc(1530, 2, 0)]);
        let h = start(Some(1));
        fail_baked_then_scene(h);
        assert!(matches!(
            machine::merge_ops(Vec::new()).as_slice(),
            [InteractReq::Loc { .. }]
        ));
        observed::post(3, |post| {
            post.locs(Vec::new());
        });
        machine::step(&mut NoJs);
        assert!(matches!(
            machine::merge_ops(Vec::new()).as_slice(),
            [InteractReq::WalkTo { .. }]
        ));

        observed::post(4, |post| {
            post.session(true).here(observed::Tile {
                x: 1,
                z: 0,
                level: 0,
            });
        });
        machine::age(h, UNSTICK_STEP_MS + 1);
        machine::step(&mut NoJs);
        assert_eq!(
            machine::take(h),
            Take::Pending,
            "closer movement resets the bounded pass instead of exhausting attempts"
        );
        assert_ne!(
            walk_token(),
            0,
            "progress restarts the baked destination walk"
        );
    }

    #[test]
    fn unstick_step_returns_as_soon_as_its_target_is_reached() {
        reset();
        post_walled_scene(vec![door_loc(1530, 2, 0)]);
        let h = start(Some(1));
        fail_baked_then_scene(h);
        assert!(matches!(
            machine::merge_ops(Vec::new()).as_slice(),
            [InteractReq::Loc { .. }]
        ));
        observed::post(3, |post| {
            post.locs(Vec::new());
        });
        machine::step(&mut NoJs);
        assert!(matches!(
            machine::merge_ops(Vec::new()).as_slice(),
            [InteractReq::WalkTo { x: 0, z: 1, .. }]
        ));
        observed::post(4, |post| {
            post.session(true).here(observed::Tile {
                x: 0,
                z: 1,
                level: 0,
            });
        });
        machine::step(&mut NoJs);
        assert_eq!(
            machine::take(h),
            Take::Settled(Outcome::Done(json!(false))),
            "arrival completes the bounded unstick pass without waiting 3000 ms"
        );
    }

    #[test]
    fn stalled_unstick_step_reclicks_before_its_bound() {
        reset();
        post_walled_scene(vec![door_loc(1530, 2, 0)]);
        let h = start(None);
        fail_baked_then_scene(h);
        assert!(matches!(
            machine::merge_ops(Vec::new()).as_slice(),
            [InteractReq::Loc { .. }]
        ));
        observed::post(3, |post| {
            post.locs(Vec::new());
        });
        machine::step(&mut NoJs);
        let first = machine::merge_ops(Vec::new());
        machine::step(&mut NoJs);
        assert!(machine::merge_ops(Vec::new()).is_empty());
        machine::step(&mut NoJs);
        assert_eq!(
            machine::merge_ops(Vec::new()),
            first,
            "a stalled direct step is re-clicked before the 3000 ms bound"
        );
        assert_eq!(machine::take(h), Take::Pending);
    }

    #[test]
    fn unstick_opens_only_the_door_that_leads_toward_dest() {
        reset();
        post_walled_scene(vec![door_loc(1531, 0, 2), door_loc(1530, 2, 0)]);
        let h = start(None);
        fail_baked_then_scene(h);
        assert_eq!(
            machine::merge_ops(Vec::new()),
            vec![InteractReq::Loc {
                x: 2,
                z: 0,
                level: 0,
                action: "Open".into(),
                id: Some(1530),
            }],
            "two doors within 3: only the one that opens a route to dest"
        );
        assert_eq!(machine::take(h), Take::Pending);
    }

    #[test]
    fn unstick_skips_an_irrelevant_door_but_takes_the_frozen_step() {
        reset();
        post_walled_scene(vec![door_loc(1531, 0, 2)]);
        let h = start(None);
        fail_baked_then_scene(h);
        assert_eq!(
            machine::merge_ops(Vec::new()),
            vec![InteractReq::WalkTo {
                x: 0,
                z: 1,
                level: 0,
            }],
            "an irrelevant door stays shut, but frozen still takes its one-tile step"
        );
        assert_eq!(machine::take(h), Take::Pending);
    }

    #[test]
    fn progress_during_no_door_unstick_step_restarts_baked() {
        reset();
        post_walled_scene(vec![door_loc(1531, 0, 2)]);
        let h = start(Some(1));
        fail_baked_then_scene(h);
        assert!(matches!(
            machine::merge_ops(Vec::new()).as_slice(),
            [InteractReq::WalkTo { x: 0, z: 1, .. }]
        ));
        observed::post(3, |post| {
            post.here(observed::Tile {
                x: 1,
                z: 0,
                level: 0,
            });
        });
        machine::age(h, UNSTICK_STEP_MS + 1);
        machine::step(&mut NoJs);
        assert_ne!(walk_token(), 0, "progress restarts the baked walk");
        assert_eq!(machine::take(h), Take::Pending);
    }

    #[test]
    fn unstick_without_a_barrier_counts_the_pass() {
        reset();
        post_here(0, 0);
        let h = start(None);
        fail_baked_then_scene(h);
        assert!(
            machine::merge_ops(Vec::new()).is_empty(),
            "no nearby door: count the completed no-progress pass as today"
        );
        assert_eq!(machine::take(h), Take::Pending);
    }

    #[test]
    fn within_radius_behind_wall_still_needs_door() {
        use client::dash3d::CollisionFlag;
        reset();
        let mut scene = counterfactual_scene(0);
        for z in -16..16 {
            counterfactual_or(&mut scene, 1, z, CollisionFlag::W_E);
            counterfactual_or(&mut scene, 2, z, CollisionFlag::W_W);
        }
        let me = WorldTile {
            x: 1,
            z: 0,
            level: 0,
        };
        let dest = WorldTile {
            x: 2,
            z: 0,
            level: 0,
        };
        let mut door = door_loc(1530, 1, 2);
        door.shape = client::dash3d::LocShape::WALL_STRAIGHT as u8;
        door.angle = client::dash3d::LocAngle::EAST as u8;
        post_counterfactual_scene(me, &scene, vec![door.clone()]);
        assert!(
            pick_helpful_door(me, dest, 1, vec![door]).is_some(),
            "the only door that makes dest reachable must count as helpful"
        );
    }

    fn start_opening(obstacles: &[&str]) -> machine::Handle {
        let args = json!({
            "dest": { "x": 10, "z": 0, "level": 0 },
            "radius": 0,
            "obstacles": obstacles,
        });
        let Started::Running(h) = machine::start("walk-opening", args, Vec::new(), 0) else {
            panic!("walk-opening runs");
        };
        h
    }

    fn opening_walk_token(dest_x: i32) -> u64 {
        match machine::merge_ops(Vec::new()).as_slice() {
            [InteractReq::Walk {
                x,
                z: 0,
                request_id,
                ..
            }] if *x == dest_x => *request_id,
            [InteractReq::WalkNear {
                x,
                z: 0,
                request_id,
                ..
            }] if *x == dest_x => *request_id,
            other => panic!("expected the opening walk to {dest_x}, got {other:?}"),
        }
    }

    #[test]
    fn walk_opening_opens_a_toward_door_after_a_failed_segment() {
        reset();
        post_walled_scene(vec![door_loc(1530, 2, 0)]);
        let h = start_opening(&["door", "gate"]);
        machine::step(&mut NoJs);
        let token = opening_walk_token(10);
        post_walk_outcome(
            1,
            token,
            WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            WorldTile {
                x: 10,
                z: 0,
                level: 0,
            },
            0,
            true,
        );
        machine::step(&mut NoJs);
        match machine::merge_ops(Vec::new()).as_slice() {
            [InteractReq::WalkNear {
                x: 2,
                z: 0,
                radius: 1,
                ..
            }] => {}
            other => {
                panic!("walkOpening walks to a path-relevant door 2 tiles away, got {other:?}")
            }
        }
        assert_eq!(machine::take(h), Take::Pending);
    }

    #[test]
    fn walk_opening_without_a_barrier_returns_false() {
        reset();
        post_here(0, 0);
        let h = start_opening(&["door", "gate"]);
        machine::step(&mut NoJs);
        let token = opening_walk_token(10);
        post_walk_outcome(
            1,
            token,
            WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            WorldTile {
                x: 10,
                z: 0,
                level: 0,
            },
            0,
            true,
        );
        machine::step(&mut NoJs);
        assert!(
            machine::merge_ops(Vec::new()).is_empty(),
            "no openable obstacle: walkOpening does not fall through to the scene ladder"
        );
        assert_eq!(machine::take(h), Take::Settled(Outcome::Done(json!(false))));
    }
}
