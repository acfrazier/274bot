use super::*;
use std::future::Future;

#[cfg(test)]
std::thread_local! {
    static ARRIVAL_LANDMARK_PREPARES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static ARRIVAL_LAYER_CLASSIFICATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static ARRIVAL_WINDOW_REPLAYS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
trait ArrivalYield {
    type YieldFuture: Future<Output = ()>;

    fn yield_unit(&mut self) -> Self::YieldFuture;
}

impl<Y, F> ArrivalYield for Y
where
    Y: FnMut() -> F,
    F: Future<Output = ()>,
{
    type YieldFuture = F;

    fn yield_unit(&mut self) -> Self::YieldFuture {
        self()
    }
}

const ARRIVAL_INIT_BATCH: usize = 1024;
const ARRIVAL_PAGE_SIDE: usize = 32;
const ARRIVAL_PAGE_CELLS: usize = ARRIVAL_PAGE_SIDE * ARRIVAL_PAGE_SIDE;
const ARRIVAL_PAGED: u16 = 1 << 10;

async fn arrival_filled_vec<T: Copy, Y: ArrivalYield>(
    len: usize,
    value: T,
    yield_unit: &mut Y,
) -> Vec<T> {
    let mut values = Vec::with_capacity(len);
    let mut batch = 0;
    for _ in 0..len {
        values.push(value);
        batch += 1;
        if batch == ARRIVAL_INIT_BATCH {
            yield_unit.yield_unit().await;
            batch = 0;
        }
    }
    if batch != 0 {
        yield_unit.yield_unit().await;
    }
    values
}

/// The full tile-coordinate rectangle in which the collision provider may
/// return `Some`. These bounds reserve memory only; they never clip reads or
/// synthesize `None`. The callback must return `None` outside this domain on
/// every level.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ArrivalCollisionBounds {
    origin_x: i32,
    origin_z: i32,
    width: usize,
    height: usize,
}

impl ArrivalCollisionBounds {
    /// Constructs a checked provider rectangle with half-open width and height.
    ///
    /// Returns `None` if its last tile would not fit in signed world coordinates.
    pub fn new(origin_x: i32, origin_z: i32, width: usize, height: usize) -> Option<Self> {
        let end_x = i64::from(origin_x).checked_add(i64::try_from(width).ok()?)?;
        let end_z = i64::from(origin_z).checked_add(i64::try_from(height).ok()?)?;
        if (width != 0 && end_x - 1 > i64::from(i32::MAX))
            || (height != 0 && end_z - 1 > i64::from(i32::MAX))
        {
            return None;
        }
        Some(Self {
            origin_x,
            origin_z,
            width,
            height,
        })
    }

    /// The minimum x coordinate in the provider rectangle.
    pub const fn origin_x(self) -> i32 {
        self.origin_x
    }

    /// The minimum z coordinate in the provider rectangle.
    pub const fn origin_z(self) -> i32 {
        self.origin_z
    }

    /// The provider rectangle's width in tiles.
    pub const fn width(self) -> usize {
        self.width
    }

    /// The provider rectangle's height in tiles.
    pub const fn height(self) -> usize {
        self.height
    }

    fn page_capacity_with_halo(self) -> Option<usize> {
        fn axis_pages(length: usize) -> Option<usize> {
            if length == 0 {
                return Some(0);
            }
            length.checked_div(ARRIVAL_PAGE_SIDE)?.checked_add(2)
        }

        // Page coordinates are query-local, so reserve the worst alignment
        // after translating the provider rectangle and adding its one-tile halo.
        axis_pages(self.width)?.checked_mul(axis_pages(self.height)?)
    }
}

fn arrival_heap_vec_bytes<T>(capacity: usize) -> Option<usize> {
    capacity.checked_mul(std::mem::size_of::<T>())
}

fn arrival_heap_hash_bytes(entry_bytes: usize, entries: usize) -> Option<usize> {
    if entries == 0 {
        return Some(0);
    }
    // HashMap reserves at a 7/8 load limit; include its power-of-two rounding.
    let required_buckets = entries.checked_mul(8)?.checked_add(6)?.checked_div(7)?;
    let buckets = required_buckets.max(16).checked_next_power_of_two()?;
    let bucket_storage = buckets.checked_mul(entry_bytes)?;
    // Include the control-group tail and old/new tables during a rehash.
    let control_storage = buckets.checked_add(16)?;
    bucket_storage.checked_add(control_storage)?.checked_mul(2)
}

fn arrival_heap_add(total: &mut usize, bytes: usize) -> Option<()> {
    *total = total.checked_add(bytes)?;
    Some(())
}

/// Returns an upper bound, in bytes, for heap storage owned by one arrival call.
///
/// The bound includes cached pages, flood arrays, candidate scratch, rounded
/// hash-table capacities, and old/new storage overlap during growth.
/// It excludes the caller's `candidates` buffer, callback captures, yield-future
/// storage, and allocator metadata. Add `candidate_capacity * size_of::<WorldTile>()`
/// and the opaque future size. Use `size_of_val` on an unpolled future.
///
/// The callback must return `None` outside `bounds` on every level. The bounds
/// reserve memory only. They do not clip collision queries. This function
/// returns `None` if checked arithmetic fails.
pub fn arrival_candidates_heap_bound(
    radius: i32,
    candidate_capacity: usize,
    bounds: ArrivalCollisionBounds,
) -> Option<usize> {
    let Ok(radius) = usize::try_from(radius) else {
        return Some(0);
    };
    if candidate_capacity == 0 {
        return Some(0);
    }
    let radius = radius.min(104);
    let goal_side = radius.checked_mul(2)?.checked_add(1)?;
    let budget = goal_side.checked_mul(goal_side)?;
    let replay_entries = budget.checked_mul(8)?.checked_add(1)?;
    let fallback = arrival_heap_hash_bytes(std::mem::size_of::<(i32, i32)>(), replay_entries)?
        .checked_add(arrival_heap_vec_bytes::<(i32, i32)>(replay_entries)?)?;
    if radius == 0 {
        return Some(fallback);
    }

    let side = radius.checked_mul(6)?.checked_add(3)?;
    let cells = side.checked_mul(side)?;
    let page_capacity = bounds.page_capacity_with_halo()?;
    let page_cells = page_capacity.checked_mul(ARRIVAL_PAGE_CELLS)?;
    let layer_queue_entries = ARRIVAL_LAYER_BATCH.checked_mul(budget.checked_add(1)?)?;
    let layer_page_count = page_capacity.min(layer_queue_entries);
    let layer_state_entries = layer_queue_entries.min(page_cells);
    let layer_state_capacity = layer_state_entries
        .checked_mul(2)?
        .checked_add(layer_page_count.checked_mul(2)?)?
        .min(page_cells);

    let mut cache_bytes = 0;
    arrival_heap_add(
        &mut cache_bytes,
        arrival_heap_vec_bytes::<Option<i32>>(cells)?,
    )?;
    arrival_heap_add(&mut cache_bytes, arrival_heap_vec_bytes::<u16>(cells)?)?;
    arrival_heap_add(
        &mut cache_bytes,
        arrival_heap_vec_bytes::<Box<ArrivalStepPage>>(page_capacity)?,
    )?;
    arrival_heap_add(
        &mut cache_bytes,
        arrival_heap_vec_bytes::<(i32, i32)>(page_capacity)?,
    )?;
    arrival_heap_add(
        &mut cache_bytes,
        arrival_heap_hash_bytes(std::mem::size_of::<((i32, i32), usize)>(), page_capacity)?,
    )?;
    let cache_pages = arrival_heap_vec_bytes::<ArrivalStepPage>(page_capacity)?;

    let stride = side.checked_add(1)?;
    let contour = stride.checked_mul(4)?;
    let exit_capacity = goal_side.checked_mul(4)?;
    let boundary_seed_capacity = side.saturating_sub(1).checked_mul(4)?;
    let mut dense_bytes = 0;
    for bytes in [
        arrival_heap_vec_bytes::<u32>(cells.checked_mul(4)?)?,
        arrival_heap_vec_bytes::<bool>(cells)?,
        arrival_heap_vec_bytes::<u8>(cells)?,
        arrival_heap_vec_bytes::<u32>(stride.checked_mul(stride)?)?,
        arrival_heap_vec_bytes::<usize>(exit_capacity)?,
    ] {
        arrival_heap_add(&mut dense_bytes, bytes)?;
    }
    let flood_queues = cells
        .checked_mul(3)?
        .checked_add(contour)?
        .checked_add(boundary_seed_capacity)?
        .checked_mul(std::mem::size_of::<usize>())?;
    let contour_depths = arrival_heap_vec_bytes::<u32>(contour)?;

    let candidate_bytes_per_entry = std::mem::size_of::<bool>()
        .checked_add(std::mem::size_of::<(usize, usize)>())?
        .checked_add(std::mem::size_of::<usize>())?
        .checked_add(std::mem::size_of::<(usize, usize)>())?
        .checked_add(std::mem::size_of::<u32>())?;
    let candidate_bytes = candidate_capacity.checked_mul(candidate_bytes_per_entry)?;

    let landmark_capacity = budget.checked_mul(2)?.checked_add(1)?;
    let landmark_bytes = arrival_heap_hash_bytes(
        std::mem::size_of::<(usize, ArrivalLandmarkCell)>(),
        landmark_capacity,
    )?
    .checked_add(arrival_heap_vec_bytes::<usize>(landmark_capacity)?)?
    .checked_add(arrival_heap_vec_bytes::<usize>(landmark_capacity)?)?;

    let replay_marks = cells
        .div_ceil(u64::BITS as usize)
        .checked_mul(std::mem::size_of::<u64>())?
        .checked_add(arrival_heap_vec_bytes::<[u64; WINDOW_MARK_WORDS]>(
            page_capacity,
        )?)?
        .checked_add(arrival_heap_vec_bytes::<usize>(replay_entries)?)?;
    let layer_base = arrival_heap_vec_bytes::<ArrivalLayerCell>(cells)?
        .checked_add(arrival_heap_vec_bytes::<usize>(cells)?)?
        .checked_add(arrival_heap_vec_bytes::<Option<Box<ArrivalLayerPage>>>(
            page_capacity,
        )?)?
        .checked_add(arrival_heap_vec_bytes::<usize>(layer_queue_entries)?)?;
    let layer_states = arrival_heap_vec_bytes::<ArrivalLayerCell>(layer_state_capacity)?
        .checked_add(arrival_heap_vec_bytes::<ArrivalLayerCell>(
            ARRIVAL_PAGE_CELLS,
        )?)?;
    let layer_pages =
        arrival_heap_vec_bytes::<ArrivalLayerPage>(layer_page_count)?.checked_add(layer_states)?;
    let phase_scratch = layer_pages.max(replay_marks);
    let mut classification_peak = cache_bytes;
    arrival_heap_add(&mut classification_peak, cache_pages)?;
    arrival_heap_add(&mut classification_peak, dense_bytes)?;
    arrival_heap_add(&mut classification_peak, candidate_bytes)?;
    arrival_heap_add(&mut classification_peak, landmark_bytes)?;
    arrival_heap_add(&mut classification_peak, layer_base)?;
    arrival_heap_add(&mut classification_peak, phase_scratch)?;
    let mut flood_peak = cache_bytes;
    arrival_heap_add(&mut flood_peak, dense_bytes)?;
    arrival_heap_add(&mut flood_peak, contour_depths)?;
    arrival_heap_add(&mut flood_peak, flood_queues)?;
    Some(classification_peak.max(flood_peak).max(fallback))
}

/// Options for `SceneQuery::can_reach` (the m8aq `SceneReachOptions`).
#[derive(Debug, Clone, Copy, Default)]
pub struct SceneReachOptions {
    /// Max BFS expansions; the m8aq default is 400 when `None`.
    pub max_steps: Option<u32>,
    /// Allow stopping one tile away from a blocked destination.
    pub adjacent_ok: bool,
}

/// The eight walk directions (N/E/S/W then the diagonals), the m8aq
/// `DIRS`. Posted `step` mask bit `i` is this entry: W E N S NW NE SW SE.
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
const ORTHO: [(i32, i32); 4] = [(-1, 0), (1, 0), (0, -1), (0, 1)];

fn step_dir_bit(dx: i32, dz: i32) -> Option<u32> {
    DIRS.iter().position(|&d| d == (dx, dz)).map(|i| i as u32)
}

fn local_in_bounds(scene: &SceneView, tile: LocalTile) -> bool {
    tile.lx >= 0 && tile.lz >= 0 && tile.lx < scene.width && tile.lz < scene.height
}

/// `f & mask == 0` over a missing flag (out of bounds reads as closed).
fn flags_open(flags: Option<i32>, mask: i32) -> bool {
    flags.is_some_and(|f| f & mask == 0)
}

/// Whether one step from `(lx, lz)` by `(dx, dz)` is clear: orthogonal
/// steps check the destination's player-walk mask, diagonals additionally
/// require both orthogonal legs (the m8aq `canStepLocal`).
#[inline]
fn can_step_local<F: Fn(i32, i32) -> Option<i32> + ?Sized>(
    flags: &F,
    lx: i32,
    lz: i32,
    dx: i32,
    dz: i32,
) -> bool {
    let (Some(nx), Some(nz)) = (lx.checked_add(dx), lz.checked_add(dz)) else {
        return false;
    };
    if dx == 0 && dz == 0 {
        return false;
    }
    if dx == 0 || dz == 0 {
        if dx == -1 {
            return flags_open(flags(nx, nz), CollisionFlag::PL_WALK_E);
        }
        if dx == 1 {
            return flags_open(flags(nx, nz), CollisionFlag::PL_WALK_W);
        }
        if dz == -1 {
            return flags_open(flags(nx, nz), CollisionFlag::PL_WALK_N);
        }
        return flags_open(flags(nx, nz), CollisionFlag::PL_WALK_S);
    }
    if dx == -1 && dz == -1 {
        return flags_open(flags(nx, nz), CollisionFlag::PL_WALK_NE)
            && can_step_local(flags, lx, lz, -1, 0)
            && can_step_local(flags, lx, lz, 0, -1);
    }
    if dx == 1 && dz == -1 {
        return flags_open(flags(nx, nz), CollisionFlag::PL_WALK_NW)
            && can_step_local(flags, lx, lz, 1, 0)
            && can_step_local(flags, lx, lz, 0, -1);
    }
    if dx == -1 && dz == 1 {
        return flags_open(flags(nx, nz), CollisionFlag::PL_WALK_SE)
            && can_step_local(flags, lx, lz, -1, 0)
            && can_step_local(flags, lx, lz, 0, 1);
    }
    flags_open(flags(nx, nz), CollisionFlag::PL_WALK_SW)
        && can_step_local(flags, lx, lz, 1, 0)
        && can_step_local(flags, lx, lz, 0, 1)
}

/// Whether the destination tile's wall mask toward `(dx, dz)` is clear
/// (the `adjacentOk` stop check).
fn can_reach_adjacent_tile<F: Fn(i32, i32) -> Option<i32> + ?Sized>(
    flags: &F,
    nx: i32,
    nz: i32,
    dx: i32,
    dz: i32,
) -> bool {
    let Some(f) = flags(nx, nz) else {
        return false;
    };
    let wall_mask = match (dx, dz) {
        (-1, 0) => CollisionFlag::W_E,
        (1, 0) => CollisionFlag::W_W,
        (0, -1) => CollisionFlag::W_N,
        (0, 1) => CollisionFlag::W_S,
        (-1, -1) => CollisionFlag::W_NE | CollisionFlag::W_N | CollisionFlag::W_E,
        (1, -1) => CollisionFlag::W_NW | CollisionFlag::W_N | CollisionFlag::W_W,
        (-1, 1) => CollisionFlag::W_SE | CollisionFlag::W_S | CollisionFlag::W_E,
        (1, 1) => CollisionFlag::W_SW | CollisionFlag::W_S | CollisionFlag::W_W,
        _ => return false,
    };
    f & wall_mask == 0
}

#[derive(Default)]
struct ReachScratch {
    seen: std::collections::HashSet<(i32, i32)>,
    queue: Vec<(i32, i32)>,
}
impl ReachScratch {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            seen: std::collections::HashSet::with_capacity(capacity),
            queue: Vec::with_capacity(capacity),
        }
    }
}

/// BFS over `can_step_local` (the m8aq `canReachLocal`).
fn can_reach_local(
    flags: &dyn Fn(i32, i32) -> Option<i32>,
    from: (i32, i32),
    to: (i32, i32),
    max_steps: u32,
    adjacent_ok: bool,
    scratch: &mut ReachScratch,
) -> bool {
    if flags(from.0, from.1).is_none() {
        return false;
    }
    scratch.seen.clear();
    scratch.queue.clear();
    scratch.seen.insert(from);
    scratch.queue.push(from);
    let mut head = 0usize;
    while head < scratch.queue.len() {
        let cur = scratch.queue[head];
        head += 1;
        if cur == to {
            return true;
        }
        if adjacent_ok
            && u64::from(cur.0.abs_diff(to.0)) + u64::from(cur.1.abs_diff(to.1)) == 1
            && can_reach_adjacent_tile(flags, to.0, to.1, to.0 - cur.0, to.1 - cur.1)
        {
            return true;
        }
        if head as u64 > u64::from(max_steps) {
            return false;
        }
        for (dx, dz) in DIRS {
            let (Some(nx), Some(nz)) = (cur.0.checked_add(dx), cur.1.checked_add(dz)) else {
                continue;
            };
            let next = (nx, nz);
            if !scratch.seen.contains(&next) && can_step_local(flags, cur.0, cur.1, dx, dz) {
                scratch.seen.insert(next);
                scratch.queue.push(next);
            }
        }
    }
    false
}

/// The built scene's collision surface plus the local player's tile
/// (the m8aq `SceneQuery`): tile mapping, flag reads, walkability and
/// reach.
pub struct SceneQuery<'a>(&'a SceneView, Option<WorldTile>);

impl<'a> SceneQuery<'a> {
    pub fn new(scene: &'a SceneView, player_tile: Option<WorldTile>) -> Self {
        SceneQuery(scene, player_tile)
    }

    /// Whether `tile` lies inside the built scene on its level.
    pub fn contains(&self, tile: WorldTile) -> bool {
        let scene = self.0;
        if !scene.available || tile.level != scene.level {
            return false;
        }
        let lx = tile.x - scene.base_x;
        let lz = tile.z - scene.base_z;
        lx >= 0 && lz >= 0 && lx < scene.width && lz < scene.height
    }

    /// The scene's build origin (m8aq `base()`).
    pub fn base(&self) -> WorldTile {
        WorldTile {
            x: self.0.base_x,
            z: self.0.base_z,
            level: self.0.level,
        }
    }

    /// The scene-local tile, `None` when the world tile is outside the
    /// scene (or on another level).
    pub fn to_local(&self, tile: WorldTile) -> Option<LocalTile> {
        if !self.contains(tile) {
            return None;
        }
        Some(LocalTile {
            lx: tile.x - self.0.base_x,
            lz: tile.z - self.0.base_z,
        })
    }

    /// The world tile of a scene-local tile, `None` when out of bounds
    /// (or the scene is unavailable).
    pub fn to_world(&self, tile: LocalTile) -> Option<WorldTile> {
        let scene = self.0;
        if !scene.available || !local_in_bounds(scene, tile) {
            return None;
        }
        Some(WorldTile {
            x: scene.base_x + tile.lx,
            z: scene.base_z + tile.lz,
            level: scene.level,
        })
    }

    /// The packed collision flags of a world tile, `None` when it is
    /// outside the scene.
    pub fn collision_at(&self, tile: WorldTile) -> Option<i32> {
        let local = self.to_local(tile)?;
        self.collision_at_local(local)
    }

    /// The packed collision flags of a scene-local tile, `None` when it
    /// is outside the scene.
    pub fn collision_at_local(&self, tile: LocalTile) -> Option<i32> {
        let scene = self.0;
        if !scene.available || !local_in_bounds(scene, tile) {
            return None;
        }
        scene
            .collision_flags
            .get((tile.lx * scene.height + tile.lz) as usize)
            .copied()
    }

    /// Whether the tile's flags are known (in-scene).
    pub fn probeable(&self, tile: WorldTile) -> bool {
        self.collision_at(tile).is_some()
    }

    /// Whether no walk-blocking flag bit is set on the tile. The client
    /// has no single `WALK_BLOCKED` const; the m8aq reference reads the
    /// packed `SQ_BLOCKED` mask (walk scenery + blocked ground/npcs).
    pub fn walkable(&self, tile: WorldTile) -> bool {
        self.collision_at(tile)
            .is_some_and(|flags| flags & CollisionFlag::SQ_BLOCKED == 0)
    }

    /// Walkable orthogonal stands whose edge into `destination` is not
    /// closed by the destination's wall mask. This is the same stop rule
    /// as an `adjacent_ok` reach probe, exposed as a zero-allocation
    /// iterator for route-goal selection.
    pub fn arrival_stands(&self, destination: WorldTile) -> impl Iterator<Item = WorldTile> + '_ {
        arrival_stands(
            destination,
            |stand| self.walkable(stand),
            |tile| self.collision_at(tile),
        )
    }

    /// Whether one adjacent step from `from` to `to` is clear (level and
    /// adjacency checked; diagonal steps need both orthogonal legs).
    pub fn can_step(&self, from: WorldTile, to: WorldTile) -> bool {
        if from.level != to.level {
            return false;
        }
        let dx = to.x - from.x;
        let dz = to.z - from.z;
        if dx.abs().max(dz.abs()) != 1 {
            return false;
        }
        let (Some(from_local), Some(_)) = (self.to_local(from), self.to_local(to)) else {
            return false;
        };
        let flags = |lx: i32, lz: i32| self.collision_at_local(LocalTile { lx, lz });
        can_step_local(&flags, from_local.lx, from_local.lz, dx, dz)
    }

    /// Whether the player can interact with `loc` from `from`; `None`
    /// when the loc's shape has no known approach model.
    pub fn can_operate_from(&self, loc: &LocView, from: WorldTile) -> Option<bool> {
        loc_approach::can_operate_from(loc, self.0, from)
    }

    /// The world tiles the player can operate `loc` from; `None` when
    /// the loc's shape has no known approach model.
    pub fn operable_tiles(&self, loc: &LocView) -> Option<Vec<WorldTile>> {
        loc_approach::operable_tiles(loc, self.0)
    }

    /// Compact bank-booth dest + readiness from `loc_approach` and an
    /// already-built flood. `None` when this loc has no footprint model.
    pub fn booth_approach(
        &self,
        loc: &LocView,
        flood: &ReachFlood,
    ) -> Option<loc_approach::BoothApproach> {
        let from = self.1?;
        loc_approach::booth_approach(loc, self.0, from, flood)
    }

    /// Whether the local player can walk to `destination` (BFS with a
    /// step budget; `adjacent_ok` stops one tile short).
    pub fn can_reach(&self, destination: WorldTile, options: &SceneReachOptions) -> bool {
        let Some(player) = self.1 else {
            return false;
        };
        if player.level != destination.level {
            return false;
        }
        let (Some(from), Some(to)) = (self.to_local(player), self.to_local(destination)) else {
            return false;
        };
        let flags = |lx: i32, lz: i32| self.collision_at_local(LocalTile { lx, lz });
        can_reach_local(
            &flags,
            (from.lx, from.lz),
            (to.lx, to.lz),
            options.max_steps.unwrap_or(400),
            options.adjacent_ok,
            &mut ReachScratch::default(),
        )
    }

    /// One scene-bounded flood from the local player. Posted isolate
    /// `reachable` / `reachable_adj` bits use this — not N× [`Self::can_reach`].
    /// No 400-step cap: the build (typically 104×104) is the bound.
    pub fn flood_reach(&self) -> Option<ReachFlood> {
        let scene = self.0;
        if !scene.available {
            return None;
        }
        let player = self.1?;
        if player.level != scene.level {
            return None;
        }
        let from = self.to_local(player)?;
        let flags = |lx: i32, lz: i32| self.collision_at_local(LocalTile { lx, lz });
        flags(from.lx, from.lz)?;
        let width = scene.width;
        let height = scene.height;
        let (Ok(width_usize), Ok(height_usize)) = (usize::try_from(width), usize::try_from(height))
        else {
            return None;
        };
        let n = width_usize.checked_mul(height_usize)?;
        if n == 0 || n > usize::from(u16::MAX) {
            return None;
        }
        let words = n.div_ceil(64);
        let mut reachable = vec![0u64; words];
        let mut reachable_adj = vec![0u64; words];
        let mut exact_rank = vec![u16::MAX; n];
        let idx = |lx: i32, lz: i32| (lx as usize) * height_usize + (lz as usize);
        let bit = |bits: &[u64], i: usize| bits[i / 64] & (1u64 << (i % 64)) != 0;
        let set = |bits: &mut [u64], i: usize| {
            bits[i / 64] |= 1u64 << (i % 64);
        };

        let mut queue = vec![(from.lx, from.lz)];
        let from_idx = idx(from.lx, from.lz);
        set(&mut reachable, from_idx);
        exact_rank[from_idx] = 0;
        let mut head = 0usize;
        while head < queue.len() {
            let (lx, lz) = queue[head];
            head += 1;
            for (dx, dz) in DIRS {
                let nx = lx + dx;
                let nz = lz + dz;
                if nx < 0 || nz < 0 || nx >= width || nz >= height {
                    continue;
                }
                let i = idx(nx, nz);
                if bit(&reachable, i) {
                    continue;
                }
                if can_step_local(&flags, lx, lz, dx, dz) {
                    set(&mut reachable, i);
                    exact_rank[i] = queue.len() as u16;
                    queue.push((nx, nz));
                }
            }
        }
        reachable_adj.copy_from_slice(&reachable);
        let mut adjacent_rank = exact_rank.clone();
        for (rank, &(lx, lz)) in queue.iter().enumerate() {
            for (dx, dz) in ORTHO {
                let nx = lx + dx;
                let nz = lz + dz;
                if nx < 0 || nz < 0 || nx >= width || nz >= height {
                    continue;
                }
                if can_reach_adjacent_tile(&flags, nx, nz, dx, dz) {
                    let i = idx(nx, nz);
                    set(&mut reachable_adj, i);
                    adjacent_rank[i] = adjacent_rank[i].min(rank as u16);
                }
            }
        }
        Some(ReachFlood {
            base_x: scene.base_x,
            base_z: scene.base_z,
            level: scene.level,
            width,
            height,
            reachable,
            reachable_adj,
            exact_rank,
            adjacent_rank,
        })
    }
}

/// Bitsets from [`SceneQuery::flood_reach`]: O(1) per entity after one BFS.
#[derive(Debug, Clone)]
pub struct ReachFlood {
    base_x: i32,
    base_z: i32,
    level: i32,
    width: i32,
    height: i32,
    reachable: Vec<u64>,
    reachable_adj: Vec<u64>,
    exact_rank: Vec<u16>,
    adjacent_rank: Vec<u16>,
}

impl ReachFlood {
    /// `(reachable, reachable_adj)` for a world tile; both false if off-scene.
    pub fn at(&self, tile: &WorldTile) -> (bool, bool) {
        if tile.level != self.level {
            return (false, false);
        }
        let lx = tile.x - self.base_x;
        let lz = tile.z - self.base_z;
        if lx < 0 || lz < 0 || lx >= self.width || lz >= self.height {
            return (false, false);
        }
        let i = (lx as usize) * (self.height as usize) + (lz as usize);
        let bit = |bits: &[u64]| bits[i / 64] & (1u64 << (i % 64)) != 0;
        (bit(&self.reachable), bit(&self.reachable_adj))
    }

    /// JS-safe u32 words for the posted isolate view. `ReachFlood` keeps
    /// u64 internally; a u64 through JS `number` would drop bits above 2^53.
    pub fn pack_u32(&self) -> (Vec<u32>, Vec<u32>) {
        let n = (self.width as usize).saturating_mul(self.height as usize);
        (
            pack_u64_bitset_to_u32(&self.reachable, n),
            pack_u64_bitset_to_u32(&self.reachable_adj, n),
        )
    }

    /// Earliest dequeue ranks for exact and exact-or-adjacent reach.
    /// `u16::MAX` marks a tile that the corresponding flood cannot reach.
    pub fn ranks(&self) -> (&[u16], &[u16]) {
        (&self.exact_rank, &self.adjacent_rank)
    }
}

/// Collision probes used by the shared walk-arrival rule. Cached scene views
/// answer from their origin's flood ranks; packed goals use the same bounded
/// collision BFS without retaining a scene or adding per-bot state.
pub trait ArrivalProbe {
    fn can_reach_from(
        &self,
        origin: WorldTile,
        destination: WorldTile,
        options: &SceneReachOptions,
    ) -> bool;
    fn walkable(&self, tile: WorldTile) -> bool;
    fn probeable(&self, tile: WorldTile) -> bool;
}

impl ArrivalProbe for ReachQueryView {
    fn can_reach_from(
        &self,
        origin: WorldTile,
        destination: WorldTile,
        options: &SceneReachOptions,
    ) -> bool {
        self.flooded_from(origin) && self.can_reach(destination, options)
    }

    fn walkable(&self, tile: WorldTile) -> bool {
        ReachQueryView::walkable(self, tile)
    }

    fn probeable(&self, tile: WorldTile) -> bool {
        ReachQueryView::probeable(self, tile)
    }
}

/// A borrowed collision surface for packed arrival goals. Scratch is reused
/// across candidate probes and dropped with this call-local adapter. At most
/// eight neighbours per permitted expansion can enter its queue/visited set.
pub struct CollisionArrivalProbe<F> {
    collision_at: F,
    scratch: std::cell::RefCell<ReachScratch>,
}

impl<F: Fn(WorldTile) -> Option<i32>> CollisionArrivalProbe<F> {
    pub fn new(collision_at: F) -> Self {
        Self {
            collision_at,
            scratch: std::cell::RefCell::new(ReachScratch::default()),
        }
    }
}

impl<F: Fn(WorldTile) -> Option<i32>> ArrivalProbe for CollisionArrivalProbe<F> {
    fn can_reach_from(
        &self,
        origin: WorldTile,
        destination: WorldTile,
        options: &SceneReachOptions,
    ) -> bool {
        if origin.level != destination.level
            || (!options.adjacent_ok && !self.walkable(destination) && origin != destination)
        {
            return false;
        }
        let flags = |x, z| {
            (self.collision_at)(WorldTile {
                x,
                z,
                level: origin.level,
            })
        };
        can_reach_local(
            &flags,
            (origin.x, origin.z),
            (destination.x, destination.z),
            options.max_steps.unwrap_or(400),
            options.adjacent_ok,
            &mut self.scratch.borrow_mut(),
        )
    }

    fn walkable(&self, tile: WorldTile) -> bool {
        flags_open((self.collision_at)(tile), CollisionFlag::SQ_BLOCKED)
    }

    fn probeable(&self, tile: WorldTile) -> bool {
        (self.collision_at)(tile).is_some()
    }
}

const WINDOW_MARK_WORDS: usize = ARRIVAL_PAGE_CELLS / u64::BITS as usize;

struct WindowReachScratch {
    dense_marks: Vec<u64>,
    page_marks: Vec<[u64; WINDOW_MARK_WORDS]>,
    queue: Vec<usize>,
}

impl WindowReachScratch {
    async fn new<Y: ArrivalYield>(
        dense_cells: usize,
        page_capacity: usize,
        queue_capacity: usize,
        yield_unit: &mut Y,
    ) -> Self {
        let dense_marks =
            arrival_filled_vec(dense_cells.div_ceil(u64::BITS as usize), 0u64, yield_unit).await;
        Self {
            dense_marks,
            page_marks: Vec::with_capacity(page_capacity),
            queue: Vec::with_capacity(queue_capacity),
        }
    }

    async fn insert<Y: ArrivalYield>(
        &mut self,
        index: usize,
        dense_cells: usize,
        yield_unit: &mut Y,
    ) -> bool {
        if index < dense_cells {
            let word = &mut self.dense_marks[index / u64::BITS as usize];
            let bit = 1 << (index % u64::BITS as usize);
            let fresh = *word & bit == 0;
            *word |= bit;
            return fresh;
        }
        let offset = index - dense_cells;
        let page_slot = offset / ARRIVAL_PAGE_CELLS;
        while self.page_marks.len() <= page_slot {
            self.page_marks.push([0; WINDOW_MARK_WORDS]);
            yield_unit.yield_unit().await;
        }
        let local = offset % ARRIVAL_PAGE_CELLS;
        let word = &mut self.page_marks[page_slot][local / u64::BITS as usize];
        let bit = 1 << (local % u64::BITS as usize);
        let fresh = *word & bit == 0;
        *word |= bit;
        fresh
    }

    fn clear_mark(&mut self, index: usize, dense_cells: usize) {
        if index < dense_cells {
            self.dense_marks[index / u64::BITS as usize] &= !(1 << (index % u64::BITS as usize));
            return;
        }
        let offset = index - dense_cells;
        let page_slot = offset / ARRIVAL_PAGE_CELLS;
        let local = offset % ARRIVAL_PAGE_CELLS;
        self.page_marks[page_slot][local / u64::BITS as usize] &=
            !(1 << (local % u64::BITS as usize));
    }
}

#[inline]
async fn window_reach<const ADJACENT: bool, Y: ArrivalYield>(
    cache: &mut ArrivalStepCache<'_, impl Fn(WorldTile) -> Option<i32>>,
    scratch: &mut WindowReachScratch,
    target: usize,
    goals: &[usize],
    budget: usize,
    yield_unit: &mut Y,
) -> (bool, usize) {
    let side = cache.side as isize;
    let offsets = [-side, side, -1, 1, -side - 1, side - 1, -side + 1, side + 1];
    let mut head = 0;
    while head < scratch.queue.len() {
        let index = scratch.queue[head];
        if index == target || (ADJACENT && goals.contains(&index)) {
            return (true, head);
        }
        if head >= budget {
            return (false, head);
        }
        let Some(mask) = cache.mask(index) else {
            break;
        };
        head += 1;
        for (bit, &offset) in offsets.iter().enumerate() {
            if mask & (1 << bit) != 0 {
                let next = index.wrapping_add_signed(offset);
                if scratch.insert(next, cache.flags.len(), yield_unit).await {
                    scratch.queue.push(next);
                }
            }
        }
        yield_unit.yield_unit().await;
    }
    (false, head)
}

/// Exact forward dequeue order over cached steps, continuing through lazy
/// dense pages outside the window. Its boundary never becomes a collision wall.
struct WindowArrivalProbe<'cache, 'input, C> {
    cache: &'cache mut ArrivalStepCache<'input, C>,
    goals: &'cache [usize],
    base: WorldTile,
    side: usize,
    scratch: WindowReachScratch,
}

impl<C: Fn(WorldTile) -> Option<i32>> WindowArrivalProbe<'_, '_, C> {
    async fn can_reach_from<const ADJACENT: bool, Y: ArrivalYield>(
        &mut self,
        origin: WorldTile,
        destination: WorldTile,
        budget: usize,
        yield_unit: &mut Y,
    ) -> bool {
        if origin.level != destination.level {
            return false;
        }
        if !ADJACENT && !self.walkable(destination) && origin != destination {
            return false;
        }
        let x = origin.x - self.base.x;
        let z = origin.z - self.base.z;
        if x < 0 || z < 0 || x >= self.side as i32 || z >= self.side as i32 {
            let flags = |lx, lz| {
                (self.cache.collision_at)(WorldTile {
                    x: lx,
                    z: lz,
                    level: origin.level,
                })
            };
            let capacity = budget.saturating_mul(8).saturating_add(1);
            let mut scratch = ReachScratch::with_capacity(capacity);
            let arrived = can_reach_local_cooperative(
                &flags,
                (origin.x, origin.z),
                (destination.x, destination.z),
                budget as u32,
                ADJACENT,
                &mut scratch,
                yield_unit,
            )
            .await;
            clear_reach_scratch(&mut scratch, yield_unit).await;
            return arrived;
        }
        let start = x as usize * self.side + z as usize;
        if !self.probeable(origin) {
            return false;
        }
        let dense_cells = self.cache.flags.len();
        self.scratch.queue.push(start);
        self.scratch.insert(start, dense_cells, yield_unit).await;
        let target_x = destination.x - self.base.x;
        let target_z = destination.z - self.base.z;
        let target = target_x as usize * self.side + target_z as usize;
        let (arrived, mut head) = if ADJACENT {
            window_reach::<true, _>(
                self.cache,
                &mut self.scratch,
                target,
                self.goals,
                budget,
                yield_unit,
            )
            .await
        } else {
            window_reach::<false, _>(
                self.cache,
                &mut self.scratch,
                target,
                self.goals,
                budget,
                yield_unit,
            )
            .await
        };
        if arrived || head >= budget || head == self.scratch.queue.len() {
            self.clear_scratch(dense_cells, yield_unit).await;
            return arrived;
        }
        while head < self.scratch.queue.len() {
            let index = self.scratch.queue[head];
            if index == target || (ADJACENT && self.goals.contains(&index)) {
                self.clear_scratch(dense_cells, yield_unit).await;
                return true;
            }
            if head >= budget {
                self.clear_scratch(dense_cells, yield_unit).await;
                return false;
            }
            head += 1;
            let mut mask = self.cache.forward_mask(index);
            while mask != 0 {
                let bit = mask.trailing_zeros() as usize;
                mask &= mask - 1;
                let next = self.cache.neighbor(index, bit);
                if self.scratch.insert(next, dense_cells, yield_unit).await {
                    self.scratch.queue.push(next);
                }
            }
            yield_unit.yield_unit().await;
        }
        self.clear_scratch(dense_cells, yield_unit).await;
        false
    }

    async fn clear_scratch<Y: ArrivalYield>(&mut self, dense_cells: usize, yield_unit: &mut Y) {
        for position in 0..self.scratch.queue.len() {
            let index = self.scratch.queue[position];
            self.scratch.clear_mark(index, dense_cells);
            yield_unit.yield_unit().await;
        }
        self.scratch.queue.clear();
    }

    fn walkable(&self, tile: WorldTile) -> bool {
        flags_open((self.cache.collision_at)(tile), CollisionFlag::SQ_BLOCKED)
    }

    fn probeable(&self, tile: WorldTile) -> bool {
        (self.cache.collision_at)(tile).is_some()
    }
}

async fn clear_reach_scratch<Y: ArrivalYield>(scratch: &mut ReachScratch, yield_unit: &mut Y) {
    if !scratch.seen.is_empty() {
        for _ in scratch.seen.drain() {
            yield_unit.yield_unit().await;
        }
    }
    scratch.queue.clear();
}

async fn can_reach_local_cooperative<F, Y: ArrivalYield>(
    flags: &F,
    from: (i32, i32),
    to: (i32, i32),
    max_steps: u32,
    adjacent_ok: bool,
    scratch: &mut ReachScratch,
    yield_unit: &mut Y,
) -> bool
where
    F: Fn(i32, i32) -> Option<i32> + ?Sized,
{
    if flags(from.0, from.1).is_none() {
        return false;
    }
    if !scratch.seen.is_empty() {
        for _ in scratch.seen.drain() {
            yield_unit.yield_unit().await;
        }
    }
    scratch.queue.clear();
    scratch.seen.insert(from);
    scratch.queue.push(from);
    let mut head = 0usize;
    while head < scratch.queue.len() {
        let cur = scratch.queue[head];
        head += 1;
        if cur == to {
            return true;
        }
        if adjacent_ok
            && u64::from(cur.0.abs_diff(to.0)) + u64::from(cur.1.abs_diff(to.1)) == 1
            && can_reach_adjacent_tile(flags, to.0, to.1, to.0 - cur.0, to.1 - cur.1)
        {
            return true;
        }
        if head as u64 > u64::from(max_steps) {
            return false;
        }
        for (dx, dz) in DIRS {
            let (Some(nx), Some(nz)) = (cur.0.checked_add(dx), cur.1.checked_add(dz)) else {
                continue;
            };
            let next = (nx, nz);
            if !scratch.seen.contains(&next) && can_step_local(flags, cur.0, cur.1, dx, dz) {
                scratch.seen.insert(next);
                scratch.queue.push(next);
            }
        }
        yield_unit.yield_unit().await;
    }
    false
}

/// Collision steps are shared by the boxed certificates and the exact floods.
/// Only the original radius box is prepared eagerly; larger floods extend this
/// call-local cache without turning its outer boundary into a wall.
struct ArrivalStepPage {
    flags: [i32; ARRIVAL_PAGE_CELLS],
    present: [u64; ARRIVAL_PAGE_CELLS / u64::BITS as usize],
    steps: [u16; ARRIVAL_PAGE_CELLS],
}

impl ArrivalStepPage {
    fn new() -> Self {
        Self {
            flags: [0; ARRIVAL_PAGE_CELLS],
            present: [0; ARRIVAL_PAGE_CELLS / u64::BITS as usize],
            steps: [ARRIVAL_PAGED; ARRIVAL_PAGE_CELLS],
        }
    }

    fn has_flags(&self, local: usize) -> bool {
        self.present[local / u64::BITS as usize] & (1 << (local % u64::BITS as usize)) != 0
    }

    fn set_has_flags(&mut self, local: usize) {
        self.present[local / u64::BITS as usize] |= 1 << (local % u64::BITS as usize);
    }
}

// Preserve the callback's auto traits instead of erasing them through dyn Fn.
struct ArrivalStepCache<'a, C> {
    flags: &'a [Option<i32>],
    steps: Vec<u16>,
    side: usize,
    base: WorldTile,
    collision_at: &'a C,
    pages: std::collections::HashMap<(i32, i32), usize>,
    page_origins: Vec<(i32, i32)>,
    page_flags: Vec<Box<ArrivalStepPage>>,
}

impl<C: Fn(WorldTile) -> Option<i32>> ArrivalStepCache<'_, C> {
    const PREPARED: u16 = 1 << 8;
    const FLAGS_LOADED: u16 = 1 << 9;
    const WALKABLE: u16 = 1 << 11;

    fn len(&self) -> usize {
        self.flags.len() + self.page_flags.len() * ARRIVAL_PAGE_CELLS
    }

    fn coordinates(&self, index: usize) -> (i32, i32) {
        if index < self.flags.len() {
            return ((index / self.side) as i32, (index % self.side) as i32);
        }
        let offset = index - self.flags.len();
        let (x, z) = self.page_origins[offset / ARRIVAL_PAGE_CELLS];
        let local = offset % ARRIVAL_PAGE_CELLS;
        (
            x + (local / ARRIVAL_PAGE_SIDE) as i32,
            z + (local % ARRIVAL_PAGE_SIDE) as i32,
        )
    }

    fn index(&mut self, x: i32, z: i32) -> usize {
        if x >= 0 && z >= 0 && x < self.side as i32 && z < self.side as i32 {
            return x as usize * self.side + z as usize;
        }
        let page_side = ARRIVAL_PAGE_SIDE as i32;
        let page = (x.div_euclid(page_side), z.div_euclid(page_side));
        let slot = if let Some(&slot) = self.pages.get(&page) {
            slot
        } else {
            let slot = self.page_origins.len();
            self.page_origins
                .push((page.0 * page_side, page.1 * page_side));
            self.page_flags.push(Box::new(ArrivalStepPage::new()));
            self.pages.insert(page, slot);
            slot
        };
        self.flags.len()
            + slot * ARRIVAL_PAGE_CELLS
            + x.rem_euclid(page_side) as usize * ARRIVAL_PAGE_SIDE
            + z.rem_euclid(page_side) as usize
    }

    fn index_if_cached(&self, x: i32, z: i32) -> Option<usize> {
        if x >= 0 && z >= 0 && x < self.side as i32 && z < self.side as i32 {
            return Some(x as usize * self.side + z as usize);
        }
        let page_side = ARRIVAL_PAGE_SIDE as i32;
        let page = (x.div_euclid(page_side), z.div_euclid(page_side));
        let slot = *self.pages.get(&page)?;
        Some(
            self.flags.len()
                + slot * ARRIVAL_PAGE_CELLS
                + x.rem_euclid(page_side) as usize * ARRIVAL_PAGE_SIDE
                + z.rem_euclid(page_side) as usize,
        )
    }

    fn step(&self, index: usize) -> u16 {
        if index < self.flags.len() {
            self.steps[index]
        } else {
            let offset = index - self.flags.len();
            self.page_flags[offset / ARRIVAL_PAGE_CELLS].steps[offset % ARRIVAL_PAGE_CELLS]
        }
    }

    fn set_step(&mut self, index: usize, step: u16) {
        if index < self.flags.len() {
            self.steps[index] = step;
        } else {
            let offset = index - self.flags.len();
            self.page_flags[offset / ARRIVAL_PAGE_CELLS].steps[offset % ARRIVAL_PAGE_CELLS] = step;
        }
    }

    fn add_step(&mut self, index: usize, step: u16) {
        self.set_step(index, self.step(index) | step);
    }

    async fn clear_paged<Y: ArrivalYield>(&mut self, yield_unit: &mut Y) {
        // Dense edge masks can reference paged neighbors, so invalidate them
        // before releasing the page indices they name. This perimeter has at
        // most 4 * 627 - 4 cells because the public radius is capped at 104.
        if !self.pages.is_empty() && self.side != 0 {
            for x in 0..self.side {
                let north = x * self.side;
                self.steps[north] = 0;
                if self.side > 1 {
                    self.steps[north + self.side - 1] = 0;
                }
            }
            if self.side > 2 {
                for z in 1..self.side - 1 {
                    self.steps[z] = 0;
                    self.steps[(self.side - 1) * self.side + z] = 0;
                }
            }
            yield_unit.yield_unit().await;
        }
        for _ in self.pages.drain() {
            yield_unit.yield_unit().await;
        }
        self.page_origins.clear();
        for _ in self.page_flags.drain(..) {
            yield_unit.yield_unit().await;
        }
        self.steps.truncate(self.flags.len());
    }

    #[inline]
    fn neighbor(&self, index: usize, bit: usize) -> usize {
        if self.step(index) & ARRIVAL_PAGED == 0 {
            let (dx, dz) = DIRS[bit];
            return index.wrapping_add_signed(dx as isize * self.side as isize + dz as isize);
        }
        let (x, z) = self.coordinates(index);
        let (dx, dz) = DIRS[bit];
        self.index_if_cached(x + dx, z + dz)
            .expect("arrival neighbor was indexed when its mask was prepared")
    }

    fn cached_flag(&self, index: usize) -> Option<Option<i32>> {
        if index < self.flags.len() {
            return Some(self.flags[index]);
        }
        let offset = index - self.flags.len();
        let (slot, local) = (offset / ARRIVAL_PAGE_CELLS, offset % ARRIVAL_PAGE_CELLS);
        let page = &self.page_flags[slot];
        if page.steps[local] & Self::FLAGS_LOADED == 0 {
            None
        } else if page.has_flags(local) {
            Some(Some(page.flags[local]))
        } else {
            Some(None)
        }
    }

    fn flag(&mut self, index: usize) -> Option<i32> {
        if index < self.flags.len() {
            return self.flags[index];
        }
        let offset = index - self.flags.len();
        let (slot, local) = (offset / ARRIVAL_PAGE_CELLS, offset % ARRIVAL_PAGE_CELLS);
        if self.page_flags[slot].steps[local] & Self::FLAGS_LOADED == 0 {
            let (x, z) = self.coordinates(index);
            let flags = self
                .base
                .x
                .checked_add(x)
                .zip(self.base.z.checked_add(z))
                .and_then(|(x, z)| {
                    (self.collision_at)(WorldTile {
                        x,
                        z,
                        level: self.base.level,
                    })
                });
            let page = &mut self.page_flags[slot];
            if let Some(flags) = flags {
                page.flags[local] = flags;
                page.set_has_flags(local);
            }
            page.steps[local] |= Self::FLAGS_LOADED;
            return flags;
        }
        self.cached_flag(index)
            .expect("loaded arrival flag remains cached")
    }

    #[inline]
    fn forward_mask(&mut self, index: usize) -> u8 {
        if self.prepared(index) {
            return self.step(index) as u8;
        }
        self.prepare_forward_mask(index)
    }

    fn prepare_forward_mask(&mut self, index: usize) -> u8 {
        if index < self.flags.len() {
            if let Some(mask) = self.mask(index) {
                return mask;
            }
        }
        self.add_step(index, ARRIVAL_PAGED);
        let mut mask = 0;
        let source_flags = self.flag(index);
        if source_flags.is_some() {
            let (x, z) = self.coordinates(index);
            let neighbors = DIRS.map(|(dx, dz)| self.index(x + dx, z + dz));
            let mut flags = [None; 8];
            for (flag, &next) in flags.iter_mut().zip(&neighbors) {
                *flag = self.flag(next);
            }
            let w = flags_open(flags[0], CollisionFlag::PL_WALK_E);
            let e = flags_open(flags[1], CollisionFlag::PL_WALK_W);
            let n = flags_open(flags[2], CollisionFlag::PL_WALK_N);
            let s = flags_open(flags[3], CollisionFlag::PL_WALK_S);
            mask = u8::from(w) | (u8::from(e) << 1) | (u8::from(n) << 2) | (u8::from(s) << 3);
            mask |= u8::from(w && n && flags_open(flags[4], CollisionFlag::PL_WALK_NE)) << 4;
            mask |= u8::from(e && n && flags_open(flags[5], CollisionFlag::PL_WALK_NW)) << 5;
            mask |= u8::from(w && s && flags_open(flags[6], CollisionFlag::PL_WALK_SE)) << 6;
            mask |= u8::from(e && s && flags_open(flags[7], CollisionFlag::PL_WALK_SW)) << 7;
        }
        self.add_step(
            index,
            u16::from(mask)
                | Self::PREPARED
                | if flags_open(source_flags, CollisionFlag::SQ_BLOCKED) {
                    Self::WALKABLE
                } else {
                    0
                },
        );
        mask
    }

    #[inline]
    fn prepared(&self, index: usize) -> bool {
        self.step(index) & Self::PREPARED != 0
    }

    #[inline]
    fn walkable(&self, index: usize) -> bool {
        if self.prepared(index) {
            self.step(index) & Self::WALKABLE != 0
        } else if index < self.flags.len() {
            flags_open(self.flags[index], CollisionFlag::SQ_BLOCKED)
        } else {
            flags_open(self.cached_flag(index).flatten(), CollisionFlag::SQ_BLOCKED)
        }
    }

    #[inline]
    fn mask(&mut self, index: usize) -> Option<u8> {
        let step = self.step(index);
        if step & ARRIVAL_PAGED != 0 {
            return None;
        }
        if step & Self::PREPARED != 0 {
            return Some(step as u8);
        }
        self.prepare_mask(index)
    }

    fn prepare_mask(&mut self, index: usize) -> Option<u8> {
        let (x, z) = (index / self.side, index % self.side);
        if x == 0 || z == 0 || x + 1 == self.side || z + 1 == self.side {
            return None;
        }
        let mut mask = 0;
        if self.flags[index].is_some() {
            let w = flags_open(self.flags[index - self.side], CollisionFlag::PL_WALK_E);
            let e = flags_open(self.flags[index + self.side], CollisionFlag::PL_WALK_W);
            let n = flags_open(self.flags[index - 1], CollisionFlag::PL_WALK_N);
            let s = flags_open(self.flags[index + 1], CollisionFlag::PL_WALK_S);
            mask = u8::from(w) | (u8::from(e) << 1) | (u8::from(n) << 2) | (u8::from(s) << 3);
            mask |= u8::from(
                w && n && flags_open(self.flags[index - self.side - 1], CollisionFlag::PL_WALK_NE),
            ) << 4;
            mask |= u8::from(
                e && n && flags_open(self.flags[index + self.side - 1], CollisionFlag::PL_WALK_NW),
            ) << 5;
            mask |= u8::from(
                w && s && flags_open(self.flags[index - self.side + 1], CollisionFlag::PL_WALK_SE),
            ) << 6;
            mask |= u8::from(
                e && s && flags_open(self.flags[index + self.side + 1], CollisionFlag::PL_WALK_SW),
            ) << 7;
        }
        self.steps[index] = u16::from(mask)
            | Self::PREPARED
            | if flags_open(self.flags[index], CollisionFlag::SQ_BLOCKED) {
                Self::WALKABLE
            } else {
                0
            };
        Some(mask)
    }
}

/// Reverse directed distances, either in the prepared radius box or in the
/// larger flag cache. The shortest boundary distance lower-bounds every path
/// that might enter the cache from unknown outside collision.
async fn arrival_distances<const BOXED: bool, Y: ArrivalYield>(
    cache: &ArrivalStepCache<'_, impl Fn(WorldTile) -> Option<i32>>,
    seeds: &[usize],
    yield_unit: &mut Y,
) -> (Vec<u32>, u32, Vec<bool>, Vec<u32>) {
    let cells = cache.flags.len();
    let mut depth = arrival_filled_vec(cells, u32::MAX, yield_unit).await;
    let mut queue = Vec::with_capacity(cells.max(seeds.len()));
    let mut connected = if BOXED {
        Vec::new()
    } else {
        arrival_filled_vec(cells, false, yield_unit).await
    };
    let mut extra = if BOXED {
        Vec::new()
    } else {
        Vec::with_capacity(cells)
    };
    for &seed in seeds {
        depth[seed] = 0;
        queue.push(seed);
        if !BOXED {
            connected[seed] = true;
        }
        yield_unit.yield_unit().await;
    }
    let flags = |x: i32, z: i32| cache.flags[x as usize * cache.side + z as usize];
    let inverse = [1, 0, 3, 2, 7, 6, 5, 4];
    let offsets = DIRS.map(|(dx, dz)| dx as isize * cache.side as isize + dz as isize);
    let mut boundary = u32::MAX;
    let mut boundary_seeds = Vec::with_capacity(if BOXED {
        0
    } else {
        cache.side.saturating_sub(1) * 4
    });
    let mut head = 0;
    while head < queue.len() {
        let index = queue[head];
        head += 1;
        let (x, z) = ((index / cache.side) as i32, (index % cache.side) as i32);
        let on_boundary =
            x == 0 || z == 0 || x + 1 == cache.side as i32 || z + 1 == cache.side as i32;
        if on_boundary {
            boundary = boundary.min(depth[index]);
            if !BOXED {
                boundary_seeds.push(index);
            }
        }
        for (bit, &(dx, dz)) in DIRS.iter().enumerate() {
            if on_boundary {
                let (px, pz) = (x + dx, z + dz);
                if px < 0 || pz < 0 || px >= cache.side as i32 || pz >= cache.side as i32 {
                    continue;
                }
            }
            let predecessor = index.wrapping_add_signed(offsets[bit]);
            if depth[predecessor] != u32::MAX
                || !cache.walkable(predecessor)
                || (BOXED && !cache.prepared(predecessor))
            {
                continue;
            }
            let allowed = if cache.prepared(predecessor) {
                cache.step(predecessor) & (1 << inverse[bit]) != 0
            } else {
                can_step_local(&flags, x + dx, z + dz, -dx, -dz)
            };
            if allowed {
                depth[predecessor] = depth[index] + 1;
                queue.push(predecessor);
                if !BOXED {
                    connected[predecessor] = true;
                }
            } else if !BOXED && !connected[predecessor] && can_step_local(&flags, x, z, dx, dz) {
                connected[predecessor] = true;
                extra.push(predecessor);
            }
        }
        yield_unit.yield_unit().await;
    }
    let outside = if BOXED || boundary == u32::MAX {
        Vec::new()
    } else {
        arrival_outside_distances(cache, &depth, &boundary_seeds, yield_unit).await
    };
    // Reverse reach alone omits forward-only dead ends. Include their weak
    // closure before using this set as a vertex-count upper bound.
    head = 0;
    while head < extra.len() {
        let index = extra[head];
        head += 1;
        if depth[index] != u32::MAX {
            continue;
        }
        let (x, z) = ((index / cache.side) as i32, (index % cache.side) as i32);
        for (bit, &(dx, dz)) in DIRS.iter().enumerate() {
            let (nx, nz) = (x + dx, z + dz);
            if nx < 0 || nz < 0 || nx >= cache.side as i32 || nz >= cache.side as i32 {
                continue;
            }
            let next = nx as usize * cache.side + nz as usize;
            if !connected[next]
                && cache.walkable(next)
                && ((if cache.prepared(index) {
                    cache.step(index) & (1 << bit) != 0
                } else {
                    can_step_local(&flags, x, z, dx, dz)
                }) || (if cache.prepared(next) {
                    cache.step(next) & (1 << inverse[bit]) != 0
                } else {
                    can_step_local(&flags, nx, nz, -dx, -dz)
                }))
            {
                connected[next] = true;
                extra.push(next);
            }
        }
        yield_unit.yield_unit().await;
    }
    (depth, boundary, connected, outside)
}

/// Collapse unknown outside collision to an open one-tile contour. Coordinate
/// clamping maps every real outside walk onto this contour without increasing
/// its length. Real directed edges remain intact inside the cache, including
/// paths that leave and re-enter several times.
async fn arrival_outside_distances<Y: ArrivalYield>(
    cache: &ArrivalStepCache<'_, impl Fn(WorldTile) -> Option<i32>>,
    depth: &[u32],
    boundary_seeds: &[usize],
    yield_unit: &mut Y,
) -> Vec<u32> {
    let side = cache.side;
    let cells = cache.flags.len();
    let span = side + 1;
    let contour_len = span * 4;
    let mut outside = arrival_filled_vec(cells, u32::MAX, yield_unit).await;
    let mut contour = arrival_filled_vec(contour_len, u32::MAX, yield_unit).await;
    let mut queue = Vec::with_capacity(cells.saturating_add(contour_len));
    let flags = |x: i32, z: i32| cache.flags[x as usize * side + z as usize];
    let contour_index = |x: i32, z: i32| {
        if x == -1 && z < side as i32 {
            (z + 1) as usize
        } else if z == side as i32 && x < side as i32 {
            span + (x + 1) as usize
        } else if x == side as i32 && z > -1 {
            span * 2 + (side as i32 - z) as usize
        } else {
            span * 3 + (side as i32 - x) as usize
        }
    };
    let offsets = DIRS.map(|(dx, dz)| dx as isize * side as isize + dz as isize);
    let inverse = [1, 0, 3, 2, 7, 6, 5, 4];
    let mut head = 0;
    let mut seed_head = 0;
    while seed_head < boundary_seeds.len() || head < queue.len() {
        let seed_depth = boundary_seeds
            .get(seed_head)
            .map_or(u32::MAX, |&index| depth[index]);
        let queued_depth = queue.get(head).map_or(u32::MAX, |&index| {
            if index < cells {
                outside[index]
            } else {
                contour[index - cells]
            }
        });
        // Reverse BFS recorded boundary seeds in distance order. Merging those
        // events with this unit-edge FIFO avoids a weighted priority queue.
        let seeded = seed_depth <= queued_depth && seed_head < boundary_seeds.len();
        let index = if seeded {
            let index = boundary_seeds[seed_head];
            seed_head += 1;
            index
        } else {
            let index = queue[head];
            head += 1;
            index
        };
        let next_depth = seed_depth.min(queued_depth).saturating_add(1);
        if index >= cells {
            let ring = index - cells;
            let offset = ring % span;
            let (x, z) = match ring / span {
                0 => (-1, offset as i32 - 1),
                1 => (offset as i32 - 1, side as i32),
                2 => (side as i32, side as i32 - offset as i32),
                _ => (side as i32 - offset as i32, -1),
            };
            for next in [
                (ring + contour_len - 1) % contour_len,
                (ring + 1) % contour_len,
                if offset == 1 {
                    (ring + contour_len - 2) % contour_len
                } else if offset + 1 == span {
                    (ring + 2) % contour_len
                } else {
                    ring
                },
            ] {
                if contour[next] == u32::MAX {
                    contour[next] = next_depth;
                    queue.push(cells + next);
                }
            }
            for &(dx, dz) in &DIRS {
                let (nx, nz) = (x + dx, z + dz);
                if nx < 0 || nz < 0 || nx >= side as i32 || nz >= side as i32 {
                    continue;
                }
                let next = nx as usize * side + nz as usize;
                if outside[next] == u32::MAX && cache.walkable(next) {
                    outside[next] = next_depth;
                    queue.push(next);
                }
            }
            yield_unit.yield_unit().await;
            continue;
        }
        let (x, z) = ((index / side) as i32, (index % side) as i32);
        let on_boundary = x == 0 || z == 0 || x + 1 == side as i32 || z + 1 == side as i32;
        for (bit, &(dx, dz)) in DIRS.iter().enumerate() {
            if on_boundary {
                let (nx, nz) = (x + dx, z + dz);
                if nx < 0 || nz < 0 || nx >= side as i32 || nz >= side as i32 {
                    let next = contour_index(nx, nz);
                    if contour[next] == u32::MAX {
                        contour[next] = next_depth;
                        queue.push(cells + next);
                    }
                    continue;
                }
            }
            // A seed denotes a known inside-only goal path. Only queued
            // vertices have already crossed the contour.
            if !seeded {
                let predecessor = index.wrapping_add_signed(offsets[bit]);
                if outside[predecessor] != u32::MAX || !cache.walkable(predecessor) {
                    continue;
                }
                // Boundary predecessors have no prepared dense-window mask.
                // Read their directed edge just as the inside-only BFS does.
                let allowed = if cache.prepared(predecessor) {
                    cache.step(predecessor) & (1 << inverse[bit]) != 0
                } else {
                    can_step_local(&flags, x + dx, z + dz, -dx, -dz)
                };
                if allowed {
                    outside[predecessor] = next_depth;
                    queue.push(predecessor);
                }
            }
        }
        yield_unit.yield_unit().await;
    }
    outside
}

/// One forward landmark ball supplies a shared rank lower bound. Its tree's
/// reversible paths let nearby origins reach every vertex in the ball before
/// their earliest possible goal. Small closed components are decided outright.
#[derive(Clone, Copy)]
struct ArrivalLandmarkCell {
    depth: u32,
    reversible: bool,
    goal_reachable: bool,
}

struct ArrivalLandmark {
    cells: std::collections::HashMap<usize, ArrivalLandmarkCell>,
    queue: Vec<usize>,
    goal_queue: Vec<usize>,
    goal_lower: u32,
}

impl ArrivalLandmark {
    fn new(budget: u32) -> Self {
        let capacity = budget as usize * 2 + 1;
        Self {
            cells: std::collections::HashMap::with_capacity(capacity),
            queue: Vec::with_capacity(capacity),
            goal_queue: Vec::with_capacity(capacity),
            goal_lower: 0,
        }
    }

    fn depth(&self, index: usize) -> u32 {
        self.cells.get(&index).map_or(u32::MAX, |cell| cell.depth)
    }

    fn reversible(&self, index: usize) -> bool {
        self.cells.get(&index).is_some_and(|cell| cell.reversible)
    }

    fn goal_reachable(&self, index: usize) -> bool {
        self.cells
            .get(&index)
            .is_some_and(|cell| cell.goal_reachable)
    }

    async fn clear<Y: ArrivalYield>(&mut self, yield_unit: &mut Y) {
        for _ in self.cells.drain() {
            yield_unit.yield_unit().await;
        }
        self.queue.clear();
        self.goal_queue.clear();
    }

    async fn prepare<Y: ArrivalYield>(
        &mut self,
        root: usize,
        budget: u32,
        goals: &[usize],
        cache: &mut ArrivalStepCache<'_, impl Fn(WorldTile) -> Option<i32>>,
        explore_no_goal: bool,
        yield_unit: &mut Y,
    ) -> (Option<u32>, u32, bool) {
        #[cfg(test)]
        ARRIVAL_LANDMARK_PREPARES.with(|count| count.set(count.get() + 1));
        self.clear(yield_unit).await;
        self.queue.push(root);
        self.cells.insert(
            root,
            ArrivalLandmarkCell {
                depth: 0,
                reversible: true,
                goal_reachable: false,
            },
        );
        let inverse = [1, 0, 3, 2, 7, 6, 5, 4];
        let mut head = 0;
        let mut ball_depth = None;
        while head < self.queue.len() {
            let index = self.queue[head];
            head += 1;
            let depth = self.depth(index);
            let reversible = self.reversible(index);
            let mut mask = cache.forward_mask(index);
            while mask != 0 {
                let bit = mask.trailing_zeros() as usize;
                mask &= mask - 1;
                let next = cache.neighbor(index, bit);
                if self.depth(next) != u32::MAX {
                    continue;
                }
                let next_depth = depth + 1;
                let next_reversible =
                    reversible && cache.forward_mask(next) & (1 << inverse[bit]) != 0;
                self.cells.insert(
                    next,
                    ArrivalLandmarkCell {
                        depth: next_depth,
                        reversible: next_reversible,
                        goal_reachable: false,
                    },
                );
                self.queue.push(next);
                if self.queue.len() > budget as usize {
                    let first_depth = *ball_depth.get_or_insert(next_depth);
                    if !explore_no_goal
                        || goals.iter().any(|&goal| self.depth(goal) != u32::MAX)
                        || self.queue.len() > budget as usize * 2
                    {
                        self.goal_lower = goals
                            .iter()
                            .map(|&goal| self.depth(goal))
                            .min()
                            .unwrap_or(u32::MAX)
                            .min(next_depth);
                        for &extra in &self.queue[budget as usize + 1..] {
                            self.cells.remove(&extra);
                            yield_unit.yield_unit().await;
                        }
                        self.queue.truncate(budget as usize + 1);
                        return (Some(first_depth), u32::MAX, false);
                    }
                }
            }
            yield_unit.yield_unit().await;
        }
        if goals.iter().any(|&goal| self.depth(goal) != u32::MAX) {
            for &goal in goals {
                if self.depth(goal) != u32::MAX {
                    if let Some(cell) = self.cells.get_mut(&goal) {
                        cell.goal_reachable = true;
                    }
                    self.goal_queue.push(goal);
                }
                yield_unit.yield_unit().await;
            }
            head = 0;
            while head < self.goal_queue.len() {
                let index = self.goal_queue[head];
                head += 1;
                for (bit, &inverse_bit) in inverse.iter().enumerate() {
                    let predecessor = cache.neighbor(index, bit);
                    if self.depth(predecessor) != u32::MAX
                        && !self.goal_reachable(predecessor)
                        && cache.step(predecessor) & (1 << inverse_bit) != 0
                    {
                        if let Some(cell) = self.cells.get_mut(&predecessor) {
                            cell.goal_reachable = true;
                        }
                        self.goal_queue.push(predecessor);
                    }
                }
                yield_unit.yield_unit().await;
            }
        }
        self.goal_lower = goals
            .iter()
            .map(|&goal| self.depth(goal))
            .min()
            .unwrap_or(u32::MAX);
        (ball_depth, u32::MAX, true)
    }
}

/// Incoming edges read and update these states together.
#[derive(Clone, Copy, Default)]
struct ArrivalLayerCell {
    low: u64,
    middle: u64,
    high: u64,
}

impl ArrivalLayerCell {
    const HISTORICAL: u8 = 1;
    const FRONTIER: u8 = 2;
    const FRONTIER_BEFORE: u8 = 3;
    const NEXT: u8 = 4;
    const NEXT_BEFORE: u8 = 5;

    fn visited(self) -> u64 {
        self.low | self.middle | self.high
    }

    fn frontier(self) -> u64 {
        self.middle & !self.high
    }

    fn next(self) -> u64 {
        self.high & !self.middle
    }

    fn before(self) -> u64 {
        self.low & self.frontier()
    }

    fn next_before(self) -> u64 {
        self.low & self.next()
    }

    fn set_state(&mut self, origins: u64, state: u8) {
        self.low = (self.low & !origins) | if state & 1 != 0 { origins } else { 0 };
        self.middle = (self.middle & !origins) | if state & 2 != 0 { origins } else { 0 };
        self.high = (self.high & !origins) | if state & 4 != 0 { origins } else { 0 };
    }
}

/// Bit-parallel forward layers for bounded cohorts of origins. Per-origin
/// vertex counts are bit-sliced, so a shared discovery increments all its
/// origins together. The eight-origin batch is the memory-bound shape: its
/// queue and state pages remain within the whole-call scratch ceiling.
/// A canonical shortest goal path orders its prefixes lexicographically in DIRS
/// order. Tracking paths before those prefixes also gives exact dequeue ranks
/// in the budget-crossing layer, without replaying each origin's flood.
const ARRIVAL_LAYER_BATCH: usize = 8;

struct ArrivalLayerPage {
    slots: [u16; ARRIVAL_PAGE_CELLS],
    states: Vec<ArrivalLayerCell>,
}

impl ArrivalLayerPage {
    fn new() -> Self {
        Self {
            slots: [u16::MAX; ARRIVAL_PAGE_CELLS],
            states: Vec::new(),
        }
    }
}

struct ArrivalLayerContext<'a> {
    goals: &'a [usize],
    budget: u32,
    goal_depth: &'a [u32],
}

struct ArrivalLayers {
    cells: Vec<ArrivalLayerCell>,
    touched: Vec<usize>,
    pages: Vec<Option<Box<ArrivalLayerPage>>>,
    queue: Vec<usize>,
}

impl ArrivalLayers {
    fn cell(&self, index: usize, dense_cells: usize) -> ArrivalLayerCell {
        if index < dense_cells {
            return self.cells[index];
        }
        let offset = index - dense_cells;
        let Some(page) = self
            .pages
            .get(offset / ARRIVAL_PAGE_CELLS)
            .and_then(Option::as_ref)
        else {
            return ArrivalLayerCell::default();
        };
        let local = offset % ARRIVAL_PAGE_CELLS;
        let slot = page.slots[local];
        if slot == u16::MAX {
            ArrivalLayerCell::default()
        } else {
            page.states[usize::from(slot)]
        }
    }

    async fn cell_mut<Y: ArrivalYield>(
        &mut self,
        index: usize,
        dense_cells: usize,
        yield_unit: &mut Y,
    ) -> &mut ArrivalLayerCell {
        if index < dense_cells {
            return &mut self.cells[index];
        }
        let offset = index - dense_cells;
        let page_slot = offset / ARRIVAL_PAGE_CELLS;
        while self.pages.len() <= page_slot {
            self.pages.push(None);
            yield_unit.yield_unit().await;
        }
        if self.pages[page_slot].is_none() {
            self.pages[page_slot] = Some(Box::new(ArrivalLayerPage::new()));
            yield_unit.yield_unit().await;
        }
        let local = offset % ARRIVAL_PAGE_CELLS;
        let needs_growth = {
            let page = self.pages[page_slot]
                .as_ref()
                .expect("the arrival layer page was initialized above");
            page.slots[local] == u16::MAX && page.states.len() == page.states.capacity()
        };
        if needs_growth {
            let page = self.pages[page_slot]
                .as_ref()
                .expect("the arrival layer page was initialized above");
            let additional = page
                .states
                .len()
                .max(4)
                .min(ARRIVAL_PAGE_CELLS - page.states.len());
            self.pages[page_slot]
                .as_mut()
                .expect("the arrival layer page was initialized above")
                .states
                .reserve_exact(additional);
            yield_unit.yield_unit().await;
        }
        let page = self.pages[page_slot]
            .as_mut()
            .expect("the arrival layer page was initialized above");
        let mut slot = page.slots[local];
        if slot == u16::MAX {
            slot = u16::try_from(page.states.len())
                .expect("arrival layer page has at most 1024 cells");
            page.states.push(ArrivalLayerCell::default());
            page.slots[local] = slot;
        }
        &mut page.states[usize::from(slot)]
    }

    async fn new<Y: ArrivalYield>(
        cells: usize,
        page_capacity: usize,
        queue_capacity: usize,
        yield_unit: &mut Y,
    ) -> Self {
        let states = arrival_filled_vec(cells, ArrivalLayerCell::default(), yield_unit).await;
        Self {
            cells: states,
            touched: Vec::with_capacity(cells),
            pages: Vec::with_capacity(page_capacity),
            queue: Vec::with_capacity(queue_capacity),
        }
    }

    async fn clear<Y: ArrivalYield>(&mut self, yield_unit: &mut Y) {
        for position in 0..self.touched.len() {
            let index = self.touched[position];
            self.cells[index] = ArrivalLayerCell::default();
            yield_unit.yield_unit().await;
        }
        self.touched.clear();
        while let Some(page) = self.pages.pop() {
            drop(page);
            yield_unit.yield_unit().await;
        }
        self.queue.clear();
    }

    async fn classify<Y: ArrivalYield>(
        &mut self,
        starts: &[usize],
        context: ArrivalLayerContext<'_>,
        canonical: u64,
        cache: &mut ArrivalStepCache<'_, impl Fn(WorldTile) -> Option<i32>>,
        yield_unit: &mut Y,
    ) -> (u64, u64) {
        let ArrivalLayerContext {
            goals,
            budget,
            goal_depth,
        } = context;
        #[cfg(test)]
        ARRIVAL_LAYER_CLASSIFICATIONS.with(|count| count.set(count.get() + 1));
        self.clear(yield_unit).await;
        let dense_cells = cache.flags.len();
        let all = u64::MAX >> (64 - starts.len());
        let mut active = all;
        let mut accepted = 0;
        let mut uncertain = 0;
        // The used encodings are unseen, historical, current/current-before,
        // and next/next-before; three planes encode all 64 origins in 24 bytes.
        let count_bits = (u32::BITS - budget.leading_zeros() + 3) as usize;
        let mut storage = [0u64; 19];
        let counts = &mut storage[..count_bits];
        let mut prior_counts = [0u64; 19];
        let mut positions = [usize::MAX; 64];
        counts[0] = all;
        for (bit, &start) in starts.iter().enumerate() {
            let origin = 1 << bit;
            if self.cell(start, dense_cells).visited() == 0 {
                if start < dense_cells {
                    self.touched.push(start);
                }
                self.queue.push(start);
            }
            self.cell_mut(start, dense_cells, yield_unit)
                .await
                .set_state(origin, ArrivalLayerCell::FRONTIER);
            if canonical & origin != 0 {
                positions[bit] = start;
            }
            yield_unit.yield_unit().await;
        }
        let mut layer_start = 0;
        let mut layer_end = self.queue.len();
        let mut previous_above = 0;
        while layer_start < layer_end {
            let at_goal = goals.iter().fold(0, |found, &goal| {
                found | self.cell(goal, dense_cells).visited()
            }) & active;
            let exact = at_goal & canonical;
            if exact != 0 {
                // Rank is zero-based: all previous layers, followed by the
                // current-layer vertices whose first shortest path precedes
                // the first shortest path to any goal.
                let mut ranks = prior_counts;
                for q in layer_start..layer_end {
                    let index = self.queue[q];
                    let mut carry = self.cell(index, dense_cells).before() & exact;
                    for plane in &mut ranks[..count_bits] {
                        let next_carry = *plane & carry;
                        *plane ^= carry;
                        carry = next_carry;
                        if carry == 0 {
                            break;
                        }
                    }
                    yield_unit.yield_unit().await;
                }
                accepted |= exact & !arrival_counts_above(&ranks[..count_bits], budget);
                active &= !exact;
            }
            let at_goal = at_goal & active;
            let above = arrival_counts_above(counts, budget);
            let within = !arrival_counts_above(counts, budget + 1);
            let yes = at_goal & within;
            let no = (at_goal & previous_above) | (active & !at_goal & above);
            let maybe = at_goal & !yes & !no;
            accepted |= yes;
            uncertain |= maybe;
            active &= !(yes | no | maybe);
            if active == 0 {
                break;
            }
            previous_above = above;
            let mut prefix_before = [0u64; 8];
            let mut next_positions = positions;
            let mut equal_positions = [(usize::MAX, 0u64); 64];
            let mut equal_len = 0;
            for (bit, &index) in positions.iter().take(starts.len()).enumerate() {
                let origin = 1 << bit;
                if canonical & active & origin != 0 {
                    let mut slot = 0;
                    while slot < equal_len && equal_positions[slot].0 < index {
                        slot += 1;
                    }
                    if slot < equal_len && equal_positions[slot].0 == index {
                        equal_positions[slot].1 |= origin;
                    } else {
                        for move_index in (slot..equal_len).rev() {
                            equal_positions[move_index + 1] = equal_positions[move_index];
                        }
                        equal_positions[slot] = (index, origin);
                        equal_len += 1;
                    }
                }
                yield_unit.yield_unit().await;
            }
            let mut prefixes = canonical & active;
            while prefixes != 0 {
                let bit = prefixes.trailing_zeros() as usize;
                prefixes &= prefixes - 1;
                let index = positions[bit];
                let mut mask = cache.forward_mask(index);
                while mask != 0 {
                    let direction = mask.trailing_zeros() as usize;
                    mask &= mask - 1;
                    let next = cache.neighbor(index, direction);
                    if next < goal_depth.len()
                        && goal_depth[next].checked_add(1) == Some(goal_depth[index])
                    {
                        // The first reducing DIRS edge is the canonical
                        // shortest-path prefix. All smaller edges precede it.
                        for before in &mut prefix_before[..direction] {
                            *before |= 1 << bit;
                        }
                        next_positions[bit] = next;
                        break;
                    }
                }
                yield_unit.yield_unit().await;
            }
            for q in layer_start..layer_end {
                let index = self.queue[q];
                let cell = self.cell(index, dense_cells);
                let origins = cell.frontier() & active;
                let before = cell.before() & origins;
                let equal = equal_positions[..equal_len]
                    .binary_search_by_key(&index, |&(at, _)| at)
                    .ok()
                    .map_or(0, |slot| equal_positions[slot].1)
                    & origins;
                self.cell_mut(index, dense_cells, yield_unit)
                    .await
                    .set_state(origins, ArrivalLayerCell::HISTORICAL);
                if origins == 0 {
                    yield_unit.yield_unit().await;
                    continue;
                }
                let mut mask = cache.forward_mask(index);
                while mask != 0 {
                    let bit = mask.trailing_zeros() as usize;
                    mask &= mask - 1;
                    let next = cache.neighbor(index, bit);
                    let cell = self.cell(next, dense_cells);
                    let visited = cell.visited();
                    let fresh = origins & !visited;
                    let prior_next = cell.next();
                    let same_layer = fresh | (origins & prior_next);
                    let next_before = same_layer & (before | (equal & prefix_before[bit]));
                    if fresh == 0 {
                        self.cell_mut(next, dense_cells, yield_unit)
                            .await
                            .set_state(next_before, ArrivalLayerCell::NEXT_BEFORE);
                        continue;
                    }
                    if visited == 0 && next < dense_cells {
                        self.touched.push(next);
                    }
                    if prior_next == 0 {
                        self.queue.push(next);
                    }
                    let cell = self.cell_mut(next, dense_cells, yield_unit).await;
                    cell.set_state(fresh, ArrivalLayerCell::NEXT);
                    cell.set_state(next_before, ArrivalLayerCell::NEXT_BEFORE);
                }
                yield_unit.yield_unit().await;
            }
            positions = next_positions;
            prior_counts[..count_bits].copy_from_slice(counts);
            for q in layer_end..self.queue.len() {
                let index = self.queue[q];
                let cell = self.cell(index, dense_cells);
                let origins = cell.next();
                let mut carry = origins;
                for plane in &mut *counts {
                    let next_carry = *plane & carry;
                    *plane ^= carry;
                    carry = next_carry;
                    if carry == 0 {
                        break;
                    }
                }
                let before = cell.next_before();
                let cell = self.cell_mut(index, dense_cells, yield_unit).await;
                cell.set_state(origins, ArrivalLayerCell::FRONTIER);
                cell.set_state(before, ArrivalLayerCell::FRONTIER_BEFORE);
                yield_unit.yield_unit().await;
            }
            layer_start = layer_end;
            layer_end = self.queue.len();
        }
        let classified = (accepted, uncertain);
        self.clear(yield_unit).await;
        classified
    }
}

/// Compare 64 bit-sliced unsigned counts with one scalar threshold.
fn arrival_counts_above(counts: &[u64], threshold: u32) -> u64 {
    let mut equal = u64::MAX;
    let mut above = 0;
    for bit in (0..counts.len()).rev() {
        if threshold & (1 << bit) == 0 {
            above |= equal & counts[bit];
            equal &= !counts[bit];
        } else {
            equal &= counts[bit];
        }
    }
    above
}

/// Weak components of the radius box only over-connect directed steps. A
/// component containing neither a goal nor an outgoing box exit is certainly
/// rejected. Other components merely group nearby origins for shared layers.
async fn arrival_components<Y: ArrivalYield>(
    cache: &ArrivalStepCache<'_, impl Fn(WorldTile) -> Option<i32>>,
    goals: &[usize],
    yield_unit: &mut Y,
) -> (Vec<u32>, Vec<u8>) {
    let dense_cells = cache.len();
    let mut components = arrival_filled_vec(dense_cells, u32::MAX, yield_unit).await;
    let mut facts = Vec::with_capacity(dense_cells);
    let mut queue = Vec::with_capacity(dense_cells);
    let offsets = DIRS.map(|(dx, dz)| dx as isize * cache.side as isize + dz as isize);
    let inverse = [1, 0, 3, 2, 7, 6, 5, 4];
    for start in 0..cache.len() {
        if !cache.prepared(start)
            || components[start] != u32::MAX
            || !flags_open(cache.flags[start], CollisionFlag::SQ_BLOCKED)
        {
            yield_unit.yield_unit().await;
            continue;
        }
        let component = facts.len() as u32;
        let mut fact = 0u8;
        queue.clear();
        queue.push(start);
        components[start] = component;
        let mut head = 0;
        while head < queue.len() {
            let index = queue[head];
            head += 1;
            fact |= u8::from(goals.contains(&index));
            for (bit, &offset) in offsets.iter().enumerate() {
                let next = index.wrapping_add_signed(offset);
                if !cache.prepared(next) {
                    fact |= u8::from(cache.step(index) & (1 << bit) != 0) << 1;
                } else if components[next] == u32::MAX
                    && flags_open(cache.flags[next], CollisionFlag::SQ_BLOCKED)
                    && (cache.step(index) & (1 << bit) != 0
                        || cache.step(next) & (1 << inverse[bit]) != 0)
                {
                    components[next] = component;
                    queue.push(next);
                }
            }
            yield_unit.yield_unit().await;
        }
        facts.push(fact);
        yield_unit.yield_unit().await;
    }
    (components, facts)
}

fn arrival_uncertain_key(
    entry: (usize, usize),
    components: &[u32],
    side: usize,
) -> (u32, usize, usize, usize) {
    let start = entry.1;
    (components[start], start / side / 8, start % side / 8, start)
}

async fn arrival_sift_down<Y: ArrivalYield>(
    entries: &mut [(usize, usize)],
    components: &[u32],
    side: usize,
    mut root: usize,
    end: usize,
    yield_unit: &mut Y,
) {
    loop {
        let child = root * 2 + 1;
        if child >= end {
            yield_unit.yield_unit().await;
            return;
        }
        let mut larger = child;
        if child + 1 < end
            && arrival_uncertain_key(entries[child], components, side)
                <= arrival_uncertain_key(entries[child + 1], components, side)
        {
            larger = child + 1;
        }
        if arrival_uncertain_key(entries[root], components, side)
            >= arrival_uncertain_key(entries[larger], components, side)
        {
            yield_unit.yield_unit().await;
            return;
        }
        entries.swap(root, larger);
        root = larger;
        yield_unit.yield_unit().await;
    }
}

async fn sort_arrival_uncertain<Y: ArrivalYield>(
    entries: &mut [(usize, usize)],
    components: &[u32],
    side: usize,
    yield_unit: &mut Y,
) {
    for start in (0..entries.len() / 2).rev() {
        arrival_sift_down(entries, components, side, start, entries.len(), yield_unit).await;
    }
    for end in (1..entries.len()).rev() {
        entries.swap(0, end);
        arrival_sift_down(entries, components, side, 0, end, yield_unit).await;
    }
}
async fn collision_probe_reach<F, Y: ArrivalYield>(
    origin: WorldTile,
    destination: WorldTile,
    options: &SceneReachOptions,
    collision_at: &F,
    scratch: &mut ReachScratch,
    yield_unit: &mut Y,
) -> bool
where
    F: Fn(WorldTile) -> Option<i32>,
{
    if origin.level != destination.level
        || (!options.adjacent_ok
            && !flags_open(collision_at(destination), CollisionFlag::SQ_BLOCKED)
            && origin != destination)
    {
        return false;
    }
    let flags = |x, z| {
        collision_at(WorldTile {
            x,
            z,
            level: origin.level,
        })
    };
    can_reach_local_cooperative(
        &flags,
        (origin.x, origin.z),
        (destination.x, destination.z),
        options.max_steps.unwrap_or(400),
        options.adjacent_ok,
        scratch,
        yield_unit,
    )
    .await
}

async fn is_arrived_collision_cooperative<F, Y: ArrivalYield>(
    me: WorldTile,
    destination: WorldTile,
    radius: i32,
    collision_at: &F,
    scratch: &mut ReachScratch,
    yield_unit: &mut Y,
) -> bool
where
    F: Fn(WorldTile) -> Option<i32>,
{
    if me.level != destination.level {
        return false;
    }
    let distance =
        me.x.abs_diff(destination.x)
            .max(me.z.abs_diff(destination.z));
    let Ok(radius) = u32::try_from(radius) else {
        return false;
    };
    if distance > radius {
        return false;
    }
    if distance == 0 {
        return true;
    }
    let side = u64::from(radius).saturating_mul(2).saturating_add(1);
    let max_steps = u32::try_from(side.saturating_mul(side)).unwrap_or(u32::MAX);
    if collision_probe_reach(
        me,
        destination,
        &SceneReachOptions {
            max_steps: Some(max_steps),
            adjacent_ok: false,
        },
        collision_at,
        scratch,
        yield_unit,
    )
    .await
    {
        return true;
    }
    if flags_open(collision_at(destination), CollisionFlag::SQ_BLOCKED) {
        return false;
    }
    if collision_at(destination).is_none() {
        return true;
    }
    collision_probe_reach(
        me,
        destination,
        &SceneReachOptions {
            max_steps: Some(max_steps),
            adjacent_ok: true,
        },
        collision_at,
        scratch,
        yield_unit,
    )
    .await
}

async fn is_arrived_window_cooperative<Y: ArrivalYield>(
    me: WorldTile,
    destination: WorldTile,
    radius: i32,
    probe: &mut WindowArrivalProbe<'_, '_, impl Fn(WorldTile) -> Option<i32>>,
    yield_unit: &mut Y,
) -> bool {
    #[cfg(test)]
    ARRIVAL_WINDOW_REPLAYS.with(|count| count.set(count.get() + 1));
    if me.level != destination.level {
        return false;
    }
    let distance =
        me.x.abs_diff(destination.x)
            .max(me.z.abs_diff(destination.z));
    let Ok(radius) = u32::try_from(radius) else {
        return false;
    };
    if distance > radius {
        return false;
    }
    if distance == 0 {
        return true;
    }
    let side = u64::from(radius) * 2 + 1;
    let max_steps = u32::try_from(side.saturating_mul(side)).unwrap_or(u32::MAX) as usize;
    if probe
        .can_reach_from::<false, _>(me, destination, max_steps, yield_unit)
        .await
    {
        return true;
    }
    if probe.walkable(destination) {
        return false;
    }
    !probe.probeable(destination)
        || probe
            .can_reach_from::<true, _>(me, destination, max_steps, yield_unit)
            .await
}

async fn retain_arrival_with_collision<F, Y: ArrivalYield>(
    candidates: &mut Vec<WorldTile>,
    destination: WorldTile,
    radius: i32,
    collision_at: &F,
    yield_unit: &mut Y,
) where
    F: Fn(WorldTile) -> Option<i32>,
{
    if candidates.is_empty() {
        return;
    }
    let side = u64::from(u32::try_from(radius).unwrap_or_default()) * 2 + 1;
    let budget = u32::try_from(side.saturating_mul(side)).unwrap_or(u32::MAX) as usize;
    let mut scratch = ReachScratch::with_capacity(budget.saturating_mul(8).saturating_add(1));
    let original_len = candidates.len();
    let mut write = 0;
    for read in 0..original_len {
        let tile = candidates[read];
        if is_arrived_collision_cooperative(
            tile,
            destination,
            radius,
            collision_at,
            &mut scratch,
            yield_unit,
        )
        .await
        {
            candidates[write] = tile;
            write += 1;
        }
        yield_unit.yield_unit().await;
    }
    clear_reach_scratch(&mut scratch, yield_unit).await;
    candidates.truncate(write);
}
/// Filter packed radius goals with exactly the [`is_arrived`] predicate.
///
/// Reverse directed floods supply goal paths across the collision cache and
/// distances to the radius box's exits, not dequeue ranks. A path before an exit
/// has at most the box's vertex count ahead of it; enclosing-square counts give
/// further safe upper bounds. Landmark balls and shared forward layers bound
/// large ranks; small or still-undecided cohorts replay the original DIRS order.
/// The cache's outer boundary is never a wall; all scratch is dropped after this call.
/// The region and expansion budget use the same radius, capped at 104.
pub fn retain_arrival_candidates(
    candidates: &mut Vec<WorldTile>,
    destination: WorldTile,
    radius: i32,
    collision_at: impl Fn(WorldTile) -> Option<i32>,
) {
    let mut yield_unit = || std::future::ready(());
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    let mut future = std::pin::pin!(retain_arrival_candidates_kernel(
        candidates,
        destination,
        radius,
        None,
        collision_at,
        &mut yield_unit,
    ));
    match future.as_mut().poll(&mut context) {
        std::task::Poll::Ready(()) => {}
        std::task::Poll::Pending => {
            panic!("the synchronous arrival driver received a pending yield")
        }
    }
}

/// Retain the candidates that satisfy the exact walk-arrival rule, yielding
/// after every bounded cache, flood, classification, replay, and retain unit.
///
/// The collision callback and candidate vector remain borrowed for the whole
/// computation. The yield callback is also borrowed; each invocation returns
/// the future awaited at the next cooperative checkpoint.
pub async fn retain_arrival_candidates_cooperative<C, Y, F>(
    candidates: &mut Vec<WorldTile>,
    destination: WorldTile,
    radius: i32,
    collision_bounds: ArrivalCollisionBounds,
    collision_at: C,
    yield_unit: &mut Y,
) where
    C: Fn(WorldTile) -> Option<i32>,
    Y: FnMut() -> F,
    F: Future<Output = ()>,
{
    retain_arrival_candidates_kernel(
        candidates,
        destination,
        radius,
        Some(collision_bounds),
        collision_at,
        yield_unit,
    )
    .await;
}

async fn retain_arrival_candidates_kernel<C, Y: ArrivalYield>(
    candidates: &mut Vec<WorldTile>,
    destination: WorldTile,
    radius: i32,
    collision_bounds: Option<ArrivalCollisionBounds>,
    collision_at: C,
    yield_unit: &mut Y,
) where
    C: Fn(WorldTile) -> Option<i32>,
{
    if radius < 0 {
        candidates.clear();
        return;
    }
    let radius = radius.min(104);
    let Some(target_flags) = collision_at(destination) else {
        retain_arrival_with_collision(candidates, destination, radius, &collision_at, yield_unit)
            .await;
        return;
    };
    if radius == 0 || candidates.is_empty() {
        retain_arrival_with_collision(candidates, destination, radius, &collision_at, yield_unit)
            .await;
        return;
    }
    let extent = radius * 3 + 1;
    let (Some(base_x), Some(base_z)) = (
        destination.x.checked_sub(extent),
        destination.z.checked_sub(extent),
    ) else {
        retain_arrival_with_collision(candidates, destination, radius, &collision_at, yield_unit)
            .await;
        return;
    };
    let side = (extent * 2 + 1) as usize;
    let cells = side * side;
    let center = extent as usize * side + extent as usize;
    let mut cached = Vec::with_capacity(cells);
    let mut steps = Vec::with_capacity(cells);
    for index in 0..cells {
        cached.push(
            base_x
                .checked_add((index / side) as i32)
                .zip(base_z.checked_add((index % side) as i32))
                .and_then(|(x, z)| {
                    collision_at(WorldTile {
                        x,
                        z,
                        level: destination.level,
                    })
                }),
        );
        steps.push(0);
        yield_unit.yield_unit().await;
    }
    let flags = |x: i32, z: i32| cached[x as usize * side + z as usize];
    let mut goals = [center; 5];
    let mut goal_len = 1;
    if target_flags & CollisionFlag::SQ_BLOCKED != 0 {
        for (dx, dz) in ORTHO {
            if can_reach_adjacent_tile(&flags, extent, extent, -dx, -dz) {
                goals[goal_len] = (extent + dx) as usize * side + (extent + dz) as usize;
                goal_len += 1;
            }
        }
    }
    let goals = &goals[..goal_len];
    let budget = (radius as u32 * 2 + 1).pow(2);
    let page_capacity = collision_bounds
        .and_then(ArrivalCollisionBounds::page_capacity_with_halo)
        .unwrap_or_default();
    let mut cache = ArrivalStepCache {
        flags: &cached,
        steps,
        side,
        base: WorldTile {
            x: base_x,
            z: base_z,
            level: destination.level,
        },
        collision_at: &collision_at,
        pages: std::collections::HashMap::with_capacity(page_capacity),
        page_origins: Vec::with_capacity(page_capacity),
        page_flags: Vec::with_capacity(page_capacity),
    };
    let lo = (extent - radius) as usize;
    let hi = (extent + radius) as usize;
    for x in lo..=hi {
        for z in lo..=hi {
            cache.mask(x * side + z);
            yield_unit.yield_unit().await;
        }
    }
    let offsets = DIRS.map(|(dx, dz)| dx as isize * side as isize + dz as isize);
    let mut exits = Vec::with_capacity((hi - lo + 1) * 4);
    for x in lo..=hi {
        for z in lo..=hi {
            if x != lo && x != hi && z != lo && z != hi {
                yield_unit.yield_unit().await;
                continue;
            }
            let index = x * side + z;
            if cache.step(index) as u8 != 0
                && offsets.iter().enumerate().any(|(bit, &offset)| {
                    cache.step(index) & (1 << bit) != 0
                        && !cache.prepared(index.wrapping_add_signed(offset))
                })
            {
                exits.push(index);
            }
            yield_unit.yield_unit().await;
        }
    }
    let exit_depth = arrival_distances::<true, _>(&cache, &exits, yield_unit)
        .await
        .0;
    let (components, component_facts) = arrival_components(&cache, goals, yield_unit).await;
    // Boxed certificates must see the original box exits. After they are
    // captured, prepare the remaining dense steps once for all reverse edges.
    for index in 0..cells {
        cache.mask(index);
        yield_unit.yield_unit().await;
    }
    let (depth, goal_boundary, connected, outside) =
        arrival_distances::<false, _>(&cache, goals, yield_unit).await;
    let outside_goal_bound = |index: usize| {
        if goal_boundary == u32::MAX {
            u32::MAX
        } else {
            outside[index]
        }
    };
    let goal_lower_bound = |index: usize| depth[index].min(outside_goal_bound(index));
    let stride = side + 1;
    let mut prefix = arrival_filled_vec(stride * stride, 0u32, yield_unit).await;
    for x in 0..side {
        let mut row = 0;
        for z in 0..side {
            row += u32::from(connected[x * side + z]);
            prefix[(x + 1) * stride + z + 1] = prefix[x * stride + z + 1] + row;
            yield_unit.yield_unit().await;
        }
    }
    let count_square = |x: usize, z: usize, reach: usize| {
        if reach > x || reach > z || x + reach >= side || z + reach >= side {
            return u32::MAX;
        }
        let (x0, z0, x1, z1) = (x - reach, z - reach, x + reach + 1, z + reach + 1);
        prefix[x1 * stride + z1] + prefix[x0 * stride + z0]
            - prefix[x0 * stride + z1]
            - prefix[x1 * stride + z0]
    };
    let mut keep = arrival_filled_vec(candidates.len(), false, yield_unit).await;
    let mut uncertain = Vec::with_capacity(candidates.len());
    let mut replay = Vec::with_capacity(candidates.len());
    for candidate in 0..candidates.len() {
        let tile = candidates[candidate];
        let distance = tile
            .x
            .abs_diff(destination.x)
            .max(tile.z.abs_diff(destination.z));
        if tile.level != destination.level || distance > radius as u32 {
            yield_unit.yield_unit().await;
            continue;
        }
        if tile == destination {
            keep[candidate] = true;
            yield_unit.yield_unit().await;
            continue;
        }
        let x = (tile.x - base_x) as usize;
        let z = (tile.z - base_z) as usize;
        let index = x * side + z;
        if cached[index].is_none() {
            yield_unit.yield_unit().await;
            continue;
        }
        if !flags_open(cached[index], CollisionFlag::SQ_BLOCKED) {
            replay.push(candidate);
            yield_unit.yield_unit().await;
            continue;
        }
        let d = depth[index];
        if d == u32::MAX && (goal_boundary == u32::MAX || outside[index] == u32::MAX) {
            yield_unit.yield_unit().await;
            continue;
        }
        if d != u32::MAX {
            let exit = exit_depth[index];
            let expansion = d.saturating_sub(exit);
            if d <= radius as u32
                || d <= exit
                || count_square(x, z, d as usize) <= budget + 1
                || count_square(
                    extent as usize,
                    extent as usize,
                    radius as usize + expansion as usize,
                ) <= budget + 1
            {
                keep[candidate] = true;
                yield_unit.yield_unit().await;
                continue;
            }
        }
        let component = components[index];
        if component != u32::MAX && component_facts[component as usize] == 0 {
            yield_unit.yield_unit().await;
            continue;
        }
        uncertain.push((candidate, index));
        yield_unit.yield_unit().await;
    }
    sort_arrival_uncertain(&mut uncertain, &components, side, yield_unit).await;
    let mut group = 0;
    let mut layers: Option<ArrivalLayers> = None;
    let mut landmark: Option<ArrivalLandmark> = None;
    let mut pending = Vec::with_capacity(candidates.len());
    let mut nearest_root = arrival_filled_vec(candidates.len(), u32::MAX, yield_unit).await;
    while group < uncertain.len() {
        let mut end = group + 1;
        while end < uncertain.len()
            && components[uncertain[end].1] == components[uncertain[group].1]
        {
            end += 1;
            yield_unit.yield_unit().await;
        }
        let region = &uncertain[group..end];
        if region.len() <= 8
            && region
                .iter()
                .all(|&(_, start)| depth[start] <= radius as u32 * 2)
        {
            for &(candidate, _) in region {
                replay.push(candidate);
                yield_unit.yield_unit().await;
            }
            group = end;
            yield_unit.yield_unit().await;
            continue;
        }
        let middle = region.len() / 2;
        let mut preferred_root = None;
        for (slot, &(_, start)) in region.iter().enumerate() {
            let bound = goal_lower_bound(start);
            if bound > radius as u32 * 2 {
                let key = (std::cmp::Reverse(bound), slot.abs_diff(middle));
                if preferred_root.is_none_or(|(_, best)| key < best) {
                    preferred_root = Some((start, key));
                }
            }
            yield_unit.yield_unit().await;
        }
        let root = if let Some((root, _)) = preferred_root {
            root
        } else {
            let mut best = (region[0].1, (depth[region[0].1], 0usize));
            for (slot, &(_, start)) in region.iter().enumerate().skip(1) {
                let key = (depth[start], slot.abs_diff(middle));
                if key < best.1 {
                    best = (start, key);
                }
                yield_unit.yield_unit().await;
            }
            best.0
        };
        let landmark = landmark.get_or_insert_with(|| ArrivalLandmark::new(budget));
        let bounds = landmark
            .prepare(
                root,
                budget,
                goals,
                &mut cache,
                depth[root] == u32::MAX,
                yield_unit,
            )
            .await;
        let mut roots = [root; 64];
        let mut root_count = 1;
        let mut undecided = |(candidate, start): (usize, usize),
                             (ball_depth, boundary, closed): (Option<u32>, u32, bool),
                             landmark: &ArrivalLandmark,
                             nearest: &mut u32| {
            let root_distance = landmark.depth(start);
            *nearest = (*nearest).min(root_distance);
            if landmark.queue[0] == start {
                keep[candidate] = goals.iter().any(|&goal| landmark.depth(goal) != u32::MAX);
                return false;
            }
            if root_distance != u32::MAX {
                if closed {
                    keep[candidate] = landmark.goal_reachable(start);
                    return false;
                }
                if let Some(ball_depth) = ball_depth {
                    let upper = root_distance.saturating_add(depth[start]);
                    if upper < ball_depth && upper <= boundary {
                        keep[candidate] = true;
                        return false;
                    }
                    if landmark.reversible(start)
                        && goal_lower_bound(start)
                            .max(landmark.goal_lower.saturating_sub(root_distance))
                            > root_distance.saturating_add(ball_depth)
                    {
                        return false;
                    }
                }
            }
            true
        };
        pending.clear();
        for &entry in region {
            if undecided(entry, bounds, landmark, &mut nearest_root[entry.0]) {
                pending.push(entry);
            }
            yield_unit.yield_unit().await;
        }
        landmark.clear(yield_unit).await;
        cache.clear_paged(yield_unit).await;
        let mut near = None;
        for &(_, start) in &pending {
            let key = depth[start];
            if near.is_none_or(|(_, best)| key < best) {
                near = Some((start, key));
            }
            yield_unit.yield_unit().await;
        }
        if let Some((near, _)) = near {
            if near != root {
                let bounds = landmark
                    .prepare(
                        near,
                        budget,
                        goals,
                        &mut cache,
                        depth[near] == u32::MAX,
                        yield_unit,
                    )
                    .await;
                roots[root_count] = near;
                root_count += 1;
                let pending_len = pending.len();
                let mut retained = 0;
                for read in 0..pending_len {
                    let entry = pending[read];
                    if undecided(entry, bounds, landmark, &mut nearest_root[entry.0]) {
                        pending[retained] = entry;
                        retained += 1;
                    }
                    yield_unit.yield_unit().await;
                }
                pending.truncate(retained);
                landmark.clear(yield_unit).await;
                cache.clear_paged(yield_unit).await;
            }
        }
        while pending.len() > 64 && root_count < roots.len() {
            let mut next_root = None;
            for &(candidate, start) in &pending {
                if !roots[..root_count].contains(&start) {
                    let distance = nearest_root[candidate];
                    if next_root.is_none_or(|(_, best)| distance >= best) {
                        next_root = Some((start, distance));
                    }
                }
                yield_unit.yield_unit().await;
            }
            let Some((root, _)) = next_root else {
                break;
            };
            roots[root_count] = root;
            root_count += 1;
            let bounds = landmark
                .prepare(
                    root,
                    budget,
                    goals,
                    &mut cache,
                    depth[root] == u32::MAX,
                    yield_unit,
                )
                .await;
            let pending_len = pending.len();
            let mut retained = 0;
            for read in 0..pending_len {
                let entry = pending[read];
                if undecided(entry, bounds, landmark, &mut nearest_root[entry.0]) {
                    pending[retained] = entry;
                    retained += 1;
                }
                yield_unit.yield_unit().await;
            }
            pending.truncate(retained);
            landmark.clear(yield_unit).await;
            cache.clear_paged(yield_unit).await;
        }
        if pending.len() <= 64 {
            for &(candidate, _) in &pending {
                replay.push(candidate);
                yield_unit.yield_unit().await;
            }
        } else {
            for chunk in pending.chunks(ARRIVAL_LAYER_BATCH) {
                let mut starts = [0; 64];
                let mut canonical = 0;
                for (bit, &(_, start)) in chunk.iter().enumerate() {
                    starts[bit] = start;
                    if depth[start] < outside_goal_bound(start) {
                        canonical |= 1 << bit;
                    }
                    yield_unit.yield_unit().await;
                }
                if layers.is_none() {
                    layers = Some(
                        ArrivalLayers::new(
                            cache.flags.len(),
                            page_capacity,
                            ARRIVAL_LAYER_BATCH * (budget as usize + 1),
                            yield_unit,
                        )
                        .await,
                    );
                }
                let (accepted, undecided) = layers
                    .as_mut()
                    .expect("arrival layers were initialized above")
                    .classify(
                        &starts[..chunk.len()],
                        ArrivalLayerContext {
                            goals,
                            budget,
                            goal_depth: &depth,
                        },
                        canonical,
                        &mut cache,
                        yield_unit,
                    )
                    .await;
                for (bit, &(candidate, _)) in chunk.iter().enumerate() {
                    keep[candidate] = accepted & (1 << bit) != 0;
                    if undecided & (1 << bit) != 0 {
                        replay.push(candidate);
                    }
                    yield_unit.yield_unit().await;
                }
                cache.clear_paged(yield_unit).await;
            }
        }
        group = end;
        yield_unit.yield_unit().await;
    }
    // Clear dense-flood pages before replay scratch overlaps them; the no-uncertain
    // path needs the same cleanup as landmark batches.
    cache.clear_paged(yield_unit).await;
    let scratch = if replay.is_empty() {
        WindowReachScratch::new(0, 0, 0, yield_unit).await
    } else {
        WindowReachScratch::new(cells, page_capacity, budget as usize * 8 + 1, yield_unit).await
    };
    let mut window_probe = WindowArrivalProbe {
        cache: &mut cache,
        goals,
        base: WorldTile {
            x: base_x,
            z: base_z,
            level: destination.level,
        },
        side,
        scratch,
    };
    for candidate in replay {
        keep[candidate] = is_arrived_window_cooperative(
            candidates[candidate],
            destination,
            radius,
            &mut window_probe,
            yield_unit,
        )
        .await;
        yield_unit.yield_unit().await;
        window_probe.cache.clear_paged(yield_unit).await;
    }
    let original_len = candidates.len();
    let mut write = 0;
    for read in 0..original_len {
        let tile = candidates[read];
        if keep[read] {
            candidates[write] = tile;
            write += 1;
        }
        yield_unit.yield_unit().await;
    }
    candidates.truncate(write);
}

/// Compact derived reach query posted on the isolate snapshot. Not a
/// scene retain and not a second flood: walkable bits come from the same
/// borrowed [`SceneView`] the flood already used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReachQueryView {
    pub available: bool,
    pub base_x: i32,
    pub base_z: i32,
    pub level: i32,
    pub width: i32,
    pub height: i32,
    pub walkable: Vec<u32>,
    pub reachable: Vec<u32>,
    pub reachable_adj: Vec<u32>,
    /// Earliest native flood dequeue rank for exact reach; `u16::MAX`
    /// means unreachable. Index is `lx * height + lz`.
    pub exact_rank: Vec<u16>,
    /// Earliest exact or wall-valid orthogonal-adjacent dequeue rank;
    /// `u16::MAX` means unreachable by either rule.
    pub adjacent_rank: Vec<u16>,
    /// One byte per in-scene tile (`lx * height + lz`); bit `i` is
    /// DIRS[i] from that tile. Empty when unavailable.
    pub step: Vec<u8>,
    /// Scene-window crop of the shared static canlight plane, packed as
    /// JS-safe u32 words at `lx * height + lz`. Empty means unavailable
    /// (distinct from a present all-zero mask of the window length).
    pub canlight: Vec<u32>,
}

impl ReachQueryView {
    /// Posted when `here` is missing or `scene.available` is false.
    /// Explicit unavailable, not an omitted table: omission would keep a
    /// previous course flood on the isolate.
    pub fn unavailable() -> Self {
        Self {
            available: false,
            base_x: 0,
            base_z: 0,
            level: 0,
            width: 0,
            height: 0,
            walkable: Vec::new(),
            reachable: Vec::new(),
            reachable_adj: Vec::new(),
            exact_rank: Vec::new(),
            adjacent_rank: Vec::new(),
            step: Vec::new(),
            canlight: Vec::new(),
        }
    }

    /// Bitset bytes for one posted view (walkable + reachable + adj + canlight).
    pub fn bitset_bytes(&self) -> usize {
        self.walkable
            .len()
            .saturating_add(self.reachable.len())
            .saturating_add(self.reachable_adj.len())
            .saturating_add(self.canlight.len())
            .saturating_mul(4)
    }

    /// Adjacent-step mask bytes (one per in-scene tile).
    pub fn step_bytes(&self) -> usize {
        self.step.len()
    }

    /// Exact plus adjacent dequeue-rank bytes (two u16 values per tile).
    pub fn rank_bytes(&self) -> usize {
        self.exact_rank
            .len()
            .saturating_add(self.adjacent_rank.len())
            .saturating_mul(2)
    }

    /// Bitsets plus step masks. Design bound, not a measured win.
    pub fn view_bytes(&self) -> usize {
        self.bitset_bytes()
            .saturating_add(self.step_bytes())
            .saturating_add(self.rank_bytes())
    }

    pub fn bit_at(
        words: &[u32],
        width: i32,
        height: i32,
        base_x: i32,
        base_z: i32,
        level: i32,
        tile: WorldTile,
    ) -> bool {
        if tile.level != level {
            return false;
        }
        let lx = tile.x - base_x;
        let lz = tile.z - base_z;
        if lx < 0 || lz < 0 || lx >= width || lz >= height {
            return false;
        }
        let i = (lx as usize) * (height as usize) + (lz as usize);
        let word = i / 32;
        let bit = i % 32;
        words.get(word).is_some_and(|w| w & (1u32 << bit) != 0)
    }

    /// Posted `canStep` lookup: same level, Chebyshev 1, then the native
    /// adjacent-step bit. Unavailable, zero-distance, off-scene `from`,
    /// and missing masks are false.
    pub fn can_step(&self, from: WorldTile, to: WorldTile) -> bool {
        if !self.available {
            return false;
        }
        if from.level != to.level || from.level != self.level {
            return false;
        }
        let Some(bit) = step_dir_bit(to.x - from.x, to.z - from.z) else {
            return false;
        };
        let lx = from.x - self.base_x;
        let lz = from.z - self.base_z;
        if lx < 0 || lz < 0 || lx >= self.width || lz >= self.height {
            return false;
        }
        let i = (lx as usize) * (self.height as usize) + (lz as usize);
        self.step.get(i).is_some_and(|m| m & (1u8 << bit) != 0)
    }

    /// Bounded O(1) coordinate reach using the native flood's dequeue
    /// order. This matches [`SceneQuery::can_reach`] without running a
    /// second BFS: exact targets and valid adjacency are checked before
    /// incrementing the expansion budget, so rank `k` succeeds at `k`.
    pub fn can_reach(&self, destination: WorldTile, options: &SceneReachOptions) -> bool {
        if !self.available || self.width <= 0 || self.height <= 0 {
            return false;
        }
        let Some(tile_count) = (self.width as usize).checked_mul(self.height as usize) else {
            return false;
        };
        if self.exact_rank.len() != tile_count || self.adjacent_rank.len() != tile_count {
            return false;
        }
        if destination.level != self.level {
            return false;
        }
        let lx = destination.x - self.base_x;
        let lz = destination.z - self.base_z;
        if lx < 0 || lz < 0 || lx >= self.width || lz >= self.height {
            return false;
        }
        let i = (lx as usize) * (self.height as usize) + (lz as usize);
        let (bits, ranks) = if options.adjacent_ok {
            (&self.reachable_adj, &self.adjacent_rank)
        } else {
            (&self.reachable, &self.exact_rank)
        };
        let bit = bits
            .get(i / 32)
            .is_some_and(|word| word & (1u32 << (i % 32)) != 0);
        let rank = ranks.get(i).copied().unwrap_or(u16::MAX);
        bit && rank != u16::MAX && u32::from(rank) <= options.max_steps.unwrap_or(400)
    }

    /// Posted `walkable`: the tile is in the window on this level and its
    /// flags carry no `SQ_BLOCKED` bit (frozen `Reachability.walkable`).
    pub fn walkable(&self, tile: WorldTile) -> bool {
        self.available
            && Self::bit_at(
                &self.walkable,
                self.width,
                self.height,
                self.base_x,
                self.base_z,
                self.level,
                tile,
            )
    }

    /// Posted `probeable`: the tile lies in the flooded window on this
    /// level, so its collision flags were read (frozen
    /// `Reachability.probeable`, `collisionFlags !== null`).
    pub fn probeable(&self, tile: WorldTile) -> bool {
        self.available && tile.level == self.level && self.local_index(tile).is_some()
    }

    /// Whether the posted flood started at `tile`: the flood gives its
    /// origin, and only its origin, dequeue rank 0.
    fn flooded_from(&self, tile: WorldTile) -> bool {
        self.available
            && tile.level == self.level
            && self.local_index(tile).and_then(|i| self.exact_rank.get(i)) == Some(&0)
    }

    fn local_index(&self, tile: WorldTile) -> Option<usize> {
        let lx = tile.x - self.base_x;
        let lz = tile.z - self.base_z;
        (lx >= 0 && lz >= 0 && lx < self.width && lz < self.height)
            .then(|| (lx as usize) * (self.height as usize) + (lz as usize))
    }
}

/// Legal cardinal approach stands, shared by live scenes and packed route goals.
/// A solid target is approachable only across an open wall edge; proximity on
/// the other side of a wall is not an interaction stand.
pub fn arrival_stands(
    destination: WorldTile,
    walkable: impl Fn(WorldTile) -> bool,
    collision_at: impl Fn(WorldTile) -> Option<i32>,
) -> impl Iterator<Item = WorldTile> {
    let approach = super::loc_approach::LocApproach {
        width: 1,
        length: 1,
        blocked_sides: 0,
    };
    ORTHO.into_iter().filter_map(move |(dx, dz)| {
        let stand = WorldTile {
            x: destination.x + dx,
            z: destination.z + dz,
            level: destination.level,
        };
        let flags = |x, z| {
            collision_at(WorldTile {
                x,
                z,
                level: destination.level,
            })
        };
        let stand_flags = collision_at(stand).unwrap_or(CollisionFlag::SQ_BLOCKED);
        (walkable(stand)
            && can_reach_adjacent_tile(&flags, destination.x, destination.z, -dx, -dz)
            && approach.can_operate(destination, stand, stand_flags))
        .then_some(stand)
    })
}

const WALL_STRAIGHT: u8 = 0;

/// A `WALL_STRAIGHT` loc's sides by angle: the facing offset, then the two
/// along-wall offsets (engine `ReachStrategy.reachWall1`).
fn straight_wall_sides(angle: u8) -> Option<[(i32, i32); 3]> {
    const WEST: u8 = 0;
    const NORTH: u8 = 1;
    const EAST: u8 = 2;
    const SOUTH: u8 = 3;

    Some(match angle {
        WEST => [(-1, 0), (0, 1), (0, -1)],
        NORTH => [(0, 1), (-1, 0), (1, 0)],
        EAST => [(1, 0), (0, 1), (0, -1)],
        SOUTH => [(0, -1), (-1, 0), (1, 0)],
        _ => return None,
    })
}

/// The tile on a `WALL_STRAIGHT` loc's angle-facing side: the doorstep across
/// the wall from the loc's own tile, from which the engine operates it.
/// `None` for other shapes or an unknown angle.
pub fn straight_wall_facing(destination: WorldTile, shape: u8, angle: u8) -> Option<WorldTile> {
    if shape != WALL_STRAIGHT {
        return None;
    }
    let [(dx, dz), _, _] = straight_wall_sides(angle)?;
    Some(WorldTile {
        x: destination.x.checked_add(dx)?,
        z: destination.z.checked_add(dz)?,
        level: destination.level,
    })
}

/// Engine `ReachStrategy.reachWall1` for `WALL_STRAIGHT`: its angle-facing
/// side is reachable across the wall; along-wall neighbours require an open
/// step into the loc tile, matching `CollisionMap.test_wall`.
pub fn straight_wall_reachable(
    from: WorldTile,
    destination: WorldTile,
    shape: u8,
    angle: u8,
    can_step: impl Fn(WorldTile, WorldTile) -> bool,
) -> bool {
    if from.level != destination.level {
        return false;
    }
    if from == destination {
        return true;
    }
    if shape != WALL_STRAIGHT {
        return false;
    }
    let at_offset = |dx: i32, dz: i32| {
        destination.x.checked_add(dx) == Some(from.x)
            && destination.z.checked_add(dz) == Some(from.z)
    };
    let Some([facing_side, along_wall_a, along_wall_b]) = straight_wall_sides(angle) else {
        return false;
    };
    at_offset(facing_side.0, facing_side.1)
        || ((at_offset(along_wall_a.0, along_wall_a.1)
            || at_offset(along_wall_b.0, along_wall_b.1))
            && can_step(from, destination))
}

/// A live door leaf offering Close is already open. Recovery may walk through
/// its passage but must never dispatch that operation to clear a route.
pub fn door_is_open<'a>(actions: impl IntoIterator<Item = &'a str>) -> bool {
    actions
        .into_iter()
        .any(|action| action.trim().eq_ignore_ascii_case("close"))
}

/// Walk arrival, shared by packed route goals, native walks and compat waits.
///
/// Same level, then Chebyshev `<= radius`; standing on `dest` arrives.
/// Otherwise, in order: exact reach arrives; a walkable but unreached dest
/// does not; an unwalkable dest arrives when it cannot be probed, or when
/// adjacent reach touches it through an open wall edge.
///
/// Both reach probes permit `(2 * radius + 1)^2` expansions: the number of
/// tiles the requested goal region can contain, saturated to the reach API's
/// budget type. Radius does not widen distance or wall tolerance.
///
/// `view` is asked only when a probe is needed (`0 < dist <= radius`).
/// Posted views read cached flood ranks in O(1), with no BFS or allocation;
/// a stale origin cannot answer either reach probe. Packed goals borrow their
/// collision surface and use the same bounded BFS as scene coordinate reach.
pub fn is_arrived<P: ArrivalProbe, V: std::ops::Deref<Target = P>>(
    me: WorldTile,
    dest: WorldTile,
    radius: i32,
    view: impl FnOnce() -> V,
) -> bool {
    if me.level != dest.level {
        return false;
    }
    let dist = me.x.abs_diff(dest.x).max(me.z.abs_diff(dest.z));
    let Ok(radius) = u32::try_from(radius) else {
        return false;
    };
    if dist > radius {
        return false;
    }
    if dist == 0 {
        return true;
    }
    let view = view();
    let side = u64::from(radius) * 2 + 1;
    let max_steps = u32::try_from(side.saturating_mul(side)).unwrap_or(u32::MAX);
    let reach = |adjacent_ok| {
        view.can_reach_from(
            me,
            dest,
            &SceneReachOptions {
                max_steps: Some(max_steps),
                adjacent_ok,
            },
        )
    };
    if reach(false) {
        return true;
    }
    if view.walkable(dest) {
        return false;
    }
    !view.probeable(dest) || reach(true)
}

/// Area arrival: the mover stands on any current-standable tile of the
/// same level within the Chebyshev `radius` of `centre`. There is no
/// path-to-centre requirement. The caller supplies the standability it
/// means: live walks pass the loaded scene's reach word, packed goals pass
/// the bake (`nav_planner::contract::CollisionView::arrived`). A negative radius
/// never arrives.
pub fn arrived_in_area(
    here: WorldTile,
    centre: WorldTile,
    radius: i32,
    standable: impl FnOnce(WorldTile) -> bool,
) -> bool {
    let Ok(radius) = u32::try_from(radius) else {
        return false;
    };
    here.level == centre.level
        && here.x.abs_diff(centre.x).max(here.z.abs_diff(centre.z)) <= radius
        && standable(here)
}

/// Pack walkable bits from a borrowed scene (`SQ_BLOCKED == 0`, same
/// `lx * height + lz` index as [`ReachFlood`]). Off-scene / missing
/// collision entries stay unset.
pub fn pack_walkable_u32(scene: &SceneView) -> Vec<u32> {
    if !scene.available || scene.width <= 0 || scene.height <= 0 {
        return Vec::new();
    }
    let width = scene.width as usize;
    let height = scene.height as usize;
    let n = width.saturating_mul(height);
    let mut words = vec![0u32; n.div_ceil(32)];
    for lx in 0..width {
        for lz in 0..height {
            let i = lx * height + lz;
            let walkable = scene
                .collision_flags
                .get(i)
                .is_some_and(|flags| flags & CollisionFlag::SQ_BLOCKED == 0);
            if walkable {
                words[i / 32] |= 1u32 << (i % 32);
            }
        }
    }
    words
}

/// One byte per in-scene tile: bit `i` is `can_step_local` along
/// `DIRS[i]`. Same borrowed [`SceneView`] as walkable packing; no
/// flood and no extra scene retain. Off-scene destinations stay unset
/// (`flags_open` on a missing flag).
pub fn pack_step_masks(scene: &SceneView) -> Vec<u8> {
    if !scene.available || scene.width <= 0 || scene.height <= 0 {
        return Vec::new();
    }
    let width = scene.width;
    let height = scene.height;
    let n = (width as usize).saturating_mul(height as usize);
    let mut masks = vec![0u8; n];
    let flags = |lx: i32, lz: i32| -> Option<i32> {
        if lx < 0 || lz < 0 || lx >= width || lz >= height {
            return None;
        }
        scene
            .collision_flags
            .get((lx as usize) * (height as usize) + (lz as usize))
            .copied()
    };
    for lx in 0..width {
        for lz in 0..height {
            let i = (lx as usize) * (height as usize) + (lz as usize);
            let mut mask = 0u8;
            for (bit, &(dx, dz)) in DIRS.iter().enumerate() {
                if can_step_local(&flags, lx, lz, dx, dz) {
                    mask |= 1u8 << (bit as u32);
                }
            }
            masks[i] = mask;
        }
    }
    masks
}

/// Borrowed world-scale static canlight plane. Index is
/// `level * width * height + z * width + x` packed 64 cells per `u64`,
/// matching the 274L sidecar. Not a scene retain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CanlightPlane<'a> {
    pub bits: &'a [u64],
    pub origin_x: i32,
    pub origin_z: i32,
    pub width: i32,
    pub height: i32,
}

/// Crop the shared world canlight plane onto the posted Reach window
/// (`lx * height + lz` u32 words). `None` or non-positive window/plane
/// geometry posts an empty vector (unavailable), not an all-lightable
/// mask. A present all-zero vector of the window length is a valid mask
/// with no eligible tiles.
pub fn pack_canlight_u32(
    base_x: i32,
    base_z: i32,
    level: i32,
    width: i32,
    height: i32,
    plane: Option<CanlightPlane<'_>>,
) -> Vec<u32> {
    let Some(plane) = plane else {
        return Vec::new();
    };
    if width <= 0 || height <= 0 || plane.width <= 0 || plane.height <= 0 {
        return Vec::new();
    }
    let n = (width as usize).saturating_mul(height as usize);
    let mut words = vec![0u32; n.div_ceil(32)];
    let plane_cells = (plane.width as usize).saturating_mul(plane.height as usize);
    if plane_cells == 0 {
        return Vec::new();
    }
    for lx in 0..width as usize {
        for lz in 0..height as usize {
            let world_lx = (base_x + lx as i32) - plane.origin_x;
            let world_lz = (base_z + lz as i32) - plane.origin_z;
            if world_lx < 0 || world_lz < 0 || world_lx >= plane.width || world_lz >= plane.height {
                continue;
            }
            if level < 0 {
                continue;
            }
            let Some(world_idx) = (level as usize).checked_mul(plane_cells).and_then(|base| {
                base.checked_add((world_lz as usize) * (plane.width as usize) + world_lx as usize)
            }) else {
                continue;
            };
            let set = plane
                .bits
                .get(world_idx / 64)
                .is_some_and(|word| word & (1u64 << (world_idx % 64)) != 0);
            if set {
                let i = lx * (height as usize) + lz;
                words[i / 32] |= 1u32 << (i % 32);
            }
        }
    }
    words
}

/// One compact query view from the scene + the flood already computed
/// for entity row bits. `flood == None` posts unavailable / empty dims.
/// Posted `level` is the flood/scene plane (the same `here.level` observe
/// already bound, including `minusedlevel`); this is not a new player-plane
/// decode. Adjacent-step masks are packed from the same scene; they are
/// not a second flood. `canlight == None` posts an empty canlight vector.
pub fn pack_reach_query(scene: &SceneView, flood: Option<&ReachFlood>) -> ReachQueryView {
    pack_reach_query_plane(scene, flood, None)
}

/// [`pack_reach_query`] plus an optional borrowed world canlight plane.
pub fn pack_reach_query_plane(
    scene: &SceneView,
    flood: Option<&ReachFlood>,
    canlight: Option<CanlightPlane<'_>>,
) -> ReachQueryView {
    let Some(flood) = flood else {
        return ReachQueryView::unavailable();
    };
    if !scene.available {
        return ReachQueryView::unavailable();
    }
    let (reachable, reachable_adj) = flood.pack_u32();
    let (exact_rank, adjacent_rank) = flood.ranks();
    ReachQueryView {
        available: true,
        base_x: flood.base_x,
        base_z: flood.base_z,
        level: flood.level,
        width: flood.width,
        height: flood.height,
        walkable: pack_walkable_u32(scene),
        reachable,
        reachable_adj,
        exact_rank: exact_rank.to_vec(),
        adjacent_rank: adjacent_rank.to_vec(),
        step: pack_step_masks(scene),
        canlight: pack_canlight_u32(
            flood.base_x,
            flood.base_z,
            flood.level,
            flood.width,
            flood.height,
            canlight,
        ),
    }
}

/// Identity for caching packed reach. Scene-static tables rebuild only when
/// [`Self::static_eq`] is false; flood ranks rebuild when the full key changes.
///
/// Loc static generation and the loc model stamp stay as separate fields so a
/// door toggle cannot XOR-alias a scenery bump (and vice versa).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReachCacheKey {
    pub scene_generation: u64,
    pub loc_static_generation: u64,
    pub loc_model_stamp: u64,
    pub available: bool,
    pub base_x: i32,
    pub base_z: i32,
    pub level: i32,
    pub width: i32,
    pub height: i32,
    pub here: Option<(i32, i32, i32)>,
    pub canlight_stamp: u64,
}

impl ReachCacheKey {
    pub fn from_parts(
        scene_generation: u64,
        loc_static_generation: u64,
        loc_model_stamp: u64,
        scene: &SceneView,
        here: Option<WorldTile>,
        canlight: Option<CanlightPlane<'_>>,
    ) -> Self {
        Self {
            scene_generation,
            loc_static_generation,
            loc_model_stamp,
            available: scene.available,
            base_x: scene.base_x,
            base_z: scene.base_z,
            level: scene.level,
            width: scene.width,
            height: scene.height,
            here: here.map(|t| (t.x, t.z, t.level)),
            canlight_stamp: canlight_stamp(canlight),
        }
    }

    pub fn static_eq(self, other: Self) -> bool {
        self.scene_generation == other.scene_generation
            && self.loc_static_generation == other.loc_static_generation
            && self.loc_model_stamp == other.loc_model_stamp
            && self.available == other.available
            && self.base_x == other.base_x
            && self.base_z == other.base_z
            && self.level == other.level
            && self.width == other.width
            && self.height == other.height
            && self.canlight_stamp == other.canlight_stamp
    }

    /// Non-zero stamp for isolate fingerprinting. `0` is reserved for
    /// posts that still fingerprint the packed vectors.
    pub fn stamp(self) -> u64 {
        let mut h = self.scene_generation.wrapping_add(1);
        h ^= self.loc_static_generation.rotate_left(7);
        h ^= self.loc_model_stamp.rotate_left(17);
        h ^= self.canlight_stamp.rotate_left(13);
        h = h.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        h ^= (self.available as u64) << 1;
        h ^= (self.base_x as u64).wrapping_mul(0x0100_0001);
        h ^= (self.base_z as u64).rotate_left(11);
        h ^= (self.level as u64) << 32;
        h ^= (self.width as u64) << 16;
        h ^= self.height as u64;
        if let Some((x, z, level)) = self.here {
            h ^= (x as u64).wrapping_mul(0x517c_c1b7);
            h ^= (z as u64).rotate_left(21);
            h ^= (level as u64) << 40;
        } else {
            h ^= 0xA5A5_A5A5_A5A5_A5A5;
        }
        h | 1
    }
}

fn canlight_stamp(plane: Option<CanlightPlane<'_>>) -> u64 {
    let Some(p) = plane else {
        return 0;
    };
    let mut h = p.bits.len() as u64;
    h ^= (p.origin_x as u64).wrapping_mul(0x9E37);
    h ^= (p.origin_z as u64).rotate_left(11);
    h ^= (p.width as u64) << 16;
    h ^= p.height as u64;
    if let Some(&w) = p.bits.first() {
        h ^= w;
    }
    if let Some(&w) = p.bits.last() {
        h ^= w.rotate_left(17);
    }
    if p.bits.len() > 2 {
        h ^= p.bits[p.bits.len() / 2];
    }
    h | 1
}

/// Per-slot cache of packed reach. Scene-static walkable/step/canlight
/// rebuild when the collision/scene generation changes; flood ranks rebuild
/// when the player tile changes on that same scene.
///
/// The packed view is `Arc` so a cache hit is a refcount bump, not a deep
/// clone. The flood is stored beside it so bank approaches reuse the BFS.
#[derive(Debug)]
pub struct ReachPackCache {
    key: Option<ReachCacheKey>,
    view: Arc<ReachQueryView>,
    flood: Option<Arc<ReachFlood>>,
    static_rebuilds: u64,
    flood_packs: u64,
}

impl Default for ReachPackCache {
    fn default() -> Self {
        Self {
            key: None,
            view: Arc::new(ReachQueryView::unavailable()),
            flood: None,
            static_rebuilds: 0,
            flood_packs: 0,
        }
    }
}

impl ReachPackCache {
    pub fn static_rebuilds(&self) -> u64 {
        self.static_rebuilds
    }

    pub fn flood_packs(&self) -> u64 {
        self.flood_packs
    }

    pub fn contains(&self, key: &ReachCacheKey) -> bool {
        self.key.as_ref() == Some(key)
    }

    pub fn view(&self) -> &ReachQueryView {
        self.view.as_ref()
    }

    pub fn view_arc(&self) -> Arc<ReachQueryView> {
        Arc::clone(&self.view)
    }

    pub fn flood_arc(&self) -> Option<Arc<ReachFlood>> {
        self.flood.clone()
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn pack(
        &mut self,
        key: ReachCacheKey,
        scene: &SceneView,
        flood: Option<Arc<ReachFlood>>,
        canlight: Option<CanlightPlane<'_>>,
    ) -> Arc<ReachQueryView> {
        if self.key.as_ref() == Some(&key) {
            return Arc::clone(&self.view);
        }
        let reuse_static = self.key.is_some_and(|k| k.static_eq(key)) && self.view.available;
        if reuse_static {
            let current =
                std::mem::replace(&mut self.view, Arc::new(ReachQueryView::unavailable()));
            let mut prev = match Arc::try_unwrap(current) {
                Ok(view) => view,
                Err(shared) => (*shared).clone(),
            };
            self.view = Arc::new(overlay_flood(&mut prev, flood.as_deref()));
            self.flood = flood;
            self.key = Some(key);
            self.flood_packs += 1;
            return Arc::clone(&self.view);
        }
        self.view = Arc::new(pack_reach_query_plane(scene, flood.as_deref(), canlight));
        self.flood = flood;
        self.key = Some(key);
        self.static_rebuilds += 1;
        self.flood_packs += 1;
        Arc::clone(&self.view)
    }
}

fn overlay_flood(prev: &mut ReachQueryView, flood: Option<&ReachFlood>) -> ReachQueryView {
    let Some(flood) = flood else {
        return ReachQueryView::unavailable();
    };
    let (reachable, reachable_adj) = flood.pack_u32();
    let (exact_rank, adjacent_rank) = flood.ranks();
    ReachQueryView {
        available: true,
        base_x: flood.base_x,
        base_z: flood.base_z,
        level: flood.level,
        width: flood.width,
        height: flood.height,
        walkable: std::mem::take(&mut prev.walkable),
        reachable,
        reachable_adj,
        exact_rank: exact_rank.to_vec(),
        adjacent_rank: adjacent_rank.to_vec(),
        step: std::mem::take(&mut prev.step),
        canlight: std::mem::take(&mut prev.canlight),
    }
}

fn pack_u64_bitset_to_u32(words: &[u64], nbits: usize) -> Vec<u32> {
    let nwords = nbits.div_ceil(32);
    let mut out = vec![0u32; nwords];
    for (i, &w) in words.iter().enumerate() {
        let lo = i * 2;
        if lo < nwords {
            out[lo] = w as u32;
        }
        let hi = lo + 1;
        if hi < nwords {
            out[hi] = (w >> 32) as u32;
        }
    }
    out
}
#[cfg(test)]
mod arrival_cooperation_tests {
    use super::*;

    fn poll_ready<F: Future>(future: F) -> F::Output {
        let mut future = std::pin::pin!(future);
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        match future.as_mut().poll(&mut context) {
            std::task::Poll::Ready(output) => output,
            std::task::Poll::Pending => panic!("ready yield callback must finish in one poll"),
        }
    }

    #[test]
    fn cooperative_future_is_send_with_shared_callbacks() {
        fn assert_send(_: impl Future<Output = ()> + Send) {}

        let destination = WorldTile {
            x: 1,
            z: 1,
            level: 0,
        };
        let mut candidates = vec![destination];
        let bounds = ArrivalCollisionBounds::new(0, 0, 3, 3).unwrap();
        let flags = Arc::new(0);
        let collision_at = move |tile: WorldTile| {
            (tile.level == 0 && (0..3).contains(&tile.x) && (0..3).contains(&tile.z))
                .then_some(*flags)
        };
        let mut yield_unit = || std::future::ready(());
        assert_send(retain_arrival_candidates_cooperative(
            &mut candidates,
            destination,
            1,
            bounds,
            collision_at,
            &mut yield_unit,
        ));
    }

    #[test]
    fn synchronous_and_cooperative_calls_accept_local_callbacks() {
        let destination = WorldTile {
            x: 1,
            z: 1,
            level: 0,
        };
        let calls = std::rc::Rc::new(std::cell::Cell::new(0));
        let collision_at = |tile: WorldTile| {
            calls.set(calls.get() + 1);
            (tile.level == 0 && (0..3).contains(&tile.x) && (0..3).contains(&tile.z)).then_some(0)
        };
        let mut sync = vec![destination];
        retain_arrival_candidates(&mut sync, destination, 1, collision_at);
        let sync_calls = calls.get();
        assert!(sync_calls > 0);

        let mut cooperative = vec![destination];
        let bounds = ArrivalCollisionBounds::new(0, 0, 3, 3).unwrap();
        let yields = std::rc::Rc::new(std::cell::Cell::new(0));
        let mut yield_unit = || {
            let yields = std::rc::Rc::clone(&yields);
            async move {
                yields.set(yields.get() + 1);
            }
        };
        poll_ready(retain_arrival_candidates_cooperative(
            &mut cooperative,
            destination,
            1,
            bounds,
            collision_at,
            &mut yield_unit,
        ));
        assert!(calls.get() > sync_calls);
        assert!(yields.get() > 0);
        assert_eq!(cooperative, sync);
    }

    #[test]
    fn window_replay_matches_exact_budget_boundary_ranks() {
        let side = 627usize;
        let cells = side * side;
        let budget = ((2 * 104 + 1) * (2 * 104 + 1)) as usize;
        let mut path = Vec::with_capacity(44_000);
        for row in 0..70 {
            let z = 2 + row * 2;
            if row == 0 {
                path.extend((1..=625).map(|x| (x, z)));
                continue;
            }
            let connector_x = if row % 2 == 1 { 625 } else { 1 };
            path.push((connector_x, z - 1));
            path.push((connector_x, z));
            if row % 2 == 1 {
                path.extend((1..625).rev().map(|x| (x, z)));
            } else {
                path.extend((2..=625).map(|x| (x, z)));
            }
        }
        assert!(path.len() > budget + 1);

        let mut flags = vec![Some(CollisionFlag::SQ_BLOCKED); cells];
        for &(x, z) in &path {
            flags[x * side + z] = Some(0);
        }
        let collision_at = |tile: WorldTile| {
            if tile.level != 0
                || tile.x < 0
                || tile.z < 0
                || tile.x >= side as i32
                || tile.z >= side as i32
            {
                return None;
            }
            flags[tile.x as usize * side + tile.z as usize]
        };
        let base = WorldTile {
            x: 0,
            z: 0,
            level: 0,
        };
        let mut cache = ArrivalStepCache {
            flags: &flags,
            steps: vec![0; cells],
            side,
            base,
            collision_at: &collision_at,
            pages: std::collections::HashMap::new(),
            page_origins: Vec::new(),
            page_flags: Vec::new(),
        };
        let origin = WorldTile {
            x: path[0].0 as i32,
            z: path[0].1 as i32,
            level: 0,
        };
        let mut yield_unit = || std::future::ready(());
        let scratch = poll_ready(WindowReachScratch::new(
            cells,
            0,
            budget * 8 + 1,
            &mut yield_unit,
        ));
        let mut probe = WindowArrivalProbe {
            cache: &mut cache,
            goals: &[],
            base,
            side,
            scratch,
        };
        for rank in [budget, budget + 1] {
            let destination = WorldTile {
                x: path[rank].0 as i32,
                z: path[rank].1 as i32,
                level: 0,
            };
            let local_flags = |x, z| collision_at(WorldTile { x, z, level: 0 });
            let mut sync_scratch = ReachScratch::with_capacity(budget * 8 + 1);
            let expected = can_reach_local(
                &local_flags,
                (origin.x, origin.z),
                (destination.x, destination.z),
                budget as u32,
                false,
                &mut sync_scratch,
            );
            assert_eq!(expected, rank == budget);
            let actual = poll_ready(probe.can_reach_from::<false, _>(
                origin,
                destination,
                budget,
                &mut yield_unit,
            ));
            assert_eq!(actual, expected, "rank={rank}");
        }
    }

    #[test]
    fn layer_batches_preserve_all_origin_counts() {
        let side = 9usize;
        let cells = side * side;
        let goal = 4 * side + 4;
        let goals = [goal];
        let budget = 81u32;
        let bounds = ArrivalCollisionBounds::new(0, 0, side, side).unwrap();
        let page_capacity = bounds.page_capacity_with_halo().unwrap();
        for origin_count in [7usize, 8, 9, 63, 64, 65] {
            let flags = vec![Some(0); cells];
            let collision_at = |tile: WorldTile| {
                if tile.level != 0
                    || tile.x < 0
                    || tile.z < 0
                    || tile.x >= side as i32
                    || tile.z >= side as i32
                {
                    return None;
                }
                Some(0)
            };
            let mut cache = ArrivalStepCache {
                flags: &flags,
                steps: vec![0; cells],
                side,
                base: WorldTile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                collision_at: &collision_at,
                pages: std::collections::HashMap::with_capacity(page_capacity),
                page_origins: Vec::with_capacity(page_capacity),
                page_flags: Vec::with_capacity(page_capacity),
            };
            let goal_depth: Vec<_> = (0..cells)
                .map(|index| {
                    let x = index / side;
                    let z = index % side;
                    x.abs_diff(goal / side).max(z.abs_diff(goal % side)) as u32
                })
                .collect();
            let starts: Vec<_> = (0..origin_count).collect();
            let mut yield_unit = || std::future::ready(());
            let mut layers = poll_ready(ArrivalLayers::new(
                cells,
                page_capacity,
                ARRIVAL_LAYER_BATCH * (budget as usize + 1),
                &mut yield_unit,
            ));
            ARRIVAL_LAYER_CLASSIFICATIONS.with(|count| count.set(0));
            for chunk in starts.chunks(ARRIVAL_LAYER_BATCH) {
                let mask = (1u64 << chunk.len()) - 1;
                let (accepted, uncertain) = poll_ready(layers.classify(
                    chunk,
                    ArrivalLayerContext {
                        goals: &goals,
                        budget,
                        goal_depth: &goal_depth,
                    },
                    mask,
                    &mut cache,
                    &mut yield_unit,
                ));
                assert_eq!(accepted | uncertain, mask, "origins={origin_count}");
                for (bit, &start) in chunk.iter().enumerate() {
                    if uncertain & (1 << bit) != 0 {
                        let origin = (start / side, start % side);
                        let goal_xy = (goal / side, goal % side);
                        let mut scratch = ReachScratch::with_capacity(budget as usize * 8 + 1);
                        assert!(can_reach_local(
                            &|x, z| collision_at(WorldTile { x, z, level: 0 }),
                            (origin.0 as i32, origin.1 as i32),
                            (goal_xy.0 as i32, goal_xy.1 as i32),
                            budget,
                            false,
                            &mut scratch,
                        ));
                    }
                }
            }
            let calls = ARRIVAL_LAYER_CLASSIFICATIONS.with(std::cell::Cell::get);
            assert_eq!(calls, origin_count.div_ceil(ARRIVAL_LAYER_BATCH));
        }
    }

    #[test]
    fn cooperative_producer_replays_blocked_candidates_in_order() {
        let destination = WorldTile {
            x: 5,
            z: 5,
            level: 0,
        };
        let blocked = WorldTile {
            x: 5,
            z: 4,
            level: 0,
        };
        let bounds = ArrivalCollisionBounds::new(0, 0, 11, 11).unwrap();
        let collision_sync = |tile: WorldTile| {
            if tile.level != 0 || !(0..11).contains(&tile.x) || !(0..11).contains(&tile.z) {
                return None;
            }
            Some(if tile == blocked {
                CollisionFlag::SQ_BLOCKED
            } else {
                0
            })
        };
        let collision_cooperative = |tile: WorldTile| {
            if tile.level != 0 || !(0..11).contains(&tile.x) || !(0..11).contains(&tile.z) {
                return None;
            }
            Some(if tile == blocked {
                CollisionFlag::SQ_BLOCKED
            } else {
                0
            })
        };
        let candidates = vec![
            blocked,
            WorldTile {
                x: 4,
                z: 5,
                level: 0,
            },
            destination,
            WorldTile {
                x: 6,
                z: 5,
                level: 0,
            },
            WorldTile {
                level: 1,
                ..destination
            },
        ];
        let mut expected = candidates.clone();
        retain_arrival_candidates(&mut expected, destination, 2, collision_sync);
        let mut actual = candidates;
        let mut yield_unit = || std::future::ready(());
        ARRIVAL_WINDOW_REPLAYS.with(|count| count.set(0));
        poll_ready(retain_arrival_candidates_cooperative(
            &mut actual,
            destination,
            2,
            bounds,
            collision_cooperative,
            &mut yield_unit,
        ));
        let replays = ARRIVAL_WINDOW_REPLAYS.with(std::cell::Cell::get);
        assert!(replays > 0);
        assert_eq!(actual, expected);
    }
}
