use super::*;
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
fn can_reach_adjacent_tile(
    flags: &dyn Fn(i32, i32) -> Option<i32>,
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

struct WindowReachScratch {
    marks: Vec<u32>,
    queue: Vec<usize>,
    epoch: u32,
}

#[inline]
fn window_reach<const ADJACENT: bool>(
    cache: &mut ArrivalStepCache<'_>,
    marks: &mut [u32],
    queue: &mut Vec<usize>,
    target: usize,
    goals: &[usize],
    budget: usize,
    epoch: u32,
) -> (bool, usize) {
    let side = cache.side as isize;
    let offsets = [-side, side, -1, 1, -side - 1, side - 1, -side + 1, side + 1];
    let mut head = 0;
    while head < queue.len() {
        let index = queue[head];
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
        let mut visit = |bit: u8, offset: isize| {
            if mask & bit != 0 {
                let next = index.wrapping_add_signed(offset);
                if marks[next] != epoch {
                    marks[next] = epoch;
                    queue.push(next);
                }
            }
        };
        visit(1, offsets[0]);
        visit(2, offsets[1]);
        visit(4, offsets[2]);
        visit(8, offsets[3]);
        visit(16, offsets[4]);
        visit(32, offsets[5]);
        visit(64, offsets[6]);
        visit(128, offsets[7]);
    }
    (false, head)
}

/// Exact forward dequeue order over cached steps, continuing through lazy
/// dense pages outside the window. Its boundary never becomes a collision wall.
struct WindowArrivalProbe<'a, F> {
    fallback: &'a CollisionArrivalProbe<F>,
    cache: std::cell::RefCell<ArrivalStepCache<'a>>,
    goals: &'a [usize],
    base: WorldTile,
    side: usize,
    scratch: std::cell::RefCell<WindowReachScratch>,
}

impl<F: Fn(WorldTile) -> Option<i32>> ArrivalProbe for WindowArrivalProbe<'_, F> {
    fn can_reach_from(
        &self,
        origin: WorldTile,
        destination: WorldTile,
        options: &SceneReachOptions,
    ) -> bool {
        if origin.level != destination.level {
            return false;
        }
        if !options.adjacent_ok && !self.walkable(destination) && origin != destination {
            return false;
        }
        let x = origin.x - self.base.x;
        let z = origin.z - self.base.z;
        if x < 0 || z < 0 || x >= self.side as i32 || z >= self.side as i32 {
            return self.fallback.can_reach_from(origin, destination, options);
        }
        let start = x as usize * self.side + z as usize;
        if !self.fallback.probeable(origin) {
            return false;
        }
        let mut scratch = self.scratch.borrow_mut();
        let mut cache = self.cache.borrow_mut();
        scratch.epoch = scratch.epoch.wrapping_add(1);
        if scratch.epoch == 0 {
            scratch.marks.fill(0);
            scratch.epoch = 1;
        }
        scratch.marks.resize(cache.steps.len(), 0);
        let epoch = scratch.epoch;
        let budget = options.max_steps.unwrap_or(400) as usize;
        scratch.queue.clear();
        scratch.queue.push(start);
        scratch.marks[start] = epoch;
        let target_x = destination.x - self.base.x;
        let target_z = destination.z - self.base.z;
        let target = target_x as usize * self.side + target_z as usize;
        let WindowReachScratch { marks, queue, .. } = &mut *scratch;
        let (arrived, mut head) = if options.adjacent_ok {
            window_reach::<true>(&mut cache, marks, queue, target, self.goals, budget, epoch)
        } else {
            window_reach::<false>(&mut cache, marks, queue, target, self.goals, budget, epoch)
        };
        if arrived || head >= budget || head == queue.len() {
            return arrived;
        }
        // Handles remain stable across page growth, preserving every queued
        // tile, visited mark and dequeue rank at the dense-window boundary.
        while head < scratch.queue.len() {
            let index = scratch.queue[head];
            if index == target || (options.adjacent_ok && self.goals.contains(&index)) {
                return true;
            }
            if head >= budget {
                return false;
            }
            head += 1;
            let mut mask = cache.forward_mask(index);
            scratch.marks.resize(cache.steps.len(), 0);
            while mask != 0 {
                let bit = mask.trailing_zeros() as usize;
                mask &= mask - 1;
                let next = cache.neighbor(index, bit);
                if scratch.marks[next] != epoch {
                    scratch.marks[next] = epoch;
                    scratch.queue.push(next);
                }
            }
        }
        false
    }

    fn walkable(&self, tile: WorldTile) -> bool {
        self.fallback.walkable(tile)
    }

    fn probeable(&self, tile: WorldTile) -> bool {
        self.fallback.probeable(tile)
    }
}

/// Collision steps are shared by the boxed certificates and the exact floods.
/// Only the original radius box is prepared eagerly; larger floods extend this
/// call-local cache without turning its outer boundary into a wall.
struct ArrivalStepCache<'a> {
    flags: &'a [Option<i32>],
    steps: Vec<u16>,
    side: usize,
    base: WorldTile,
    collision_at: &'a dyn Fn(WorldTile) -> Option<i32>,
    pages: std::collections::HashMap<(i32, i32), usize>,
    page_origins: Vec<(i32, i32)>,
    page_flags: Vec<Option<i32>>,
    page_neighbors: Vec<[usize; 8]>,
    boundary_neighbors: Vec<[usize; 8]>,
}

impl ArrivalStepCache<'_> {
    const PREPARED: u16 = 1 << 8;
    const FLAGS_LOADED: u16 = 1 << 9;
    const PAGED: u16 = 1 << 10;
    const WALKABLE: u16 = 1 << 11;
    const PAGE_SIDE: usize = 32;
    const PAGE_CELLS: usize = Self::PAGE_SIDE * Self::PAGE_SIDE;

    fn coordinates(&self, index: usize) -> (i32, i32) {
        if index < self.flags.len() {
            return ((index / self.side) as i32, (index % self.side) as i32);
        }
        let offset = index - self.flags.len();
        let (x, z) = self.page_origins[offset / Self::PAGE_CELLS];
        let local = offset % Self::PAGE_CELLS;
        (
            x + (local / Self::PAGE_SIDE) as i32,
            z + (local % Self::PAGE_SIDE) as i32,
        )
    }

    fn index(&mut self, x: i32, z: i32) -> usize {
        if x >= 0 && z >= 0 && x < self.side as i32 && z < self.side as i32 {
            return x as usize * self.side + z as usize;
        }
        let page_side = Self::PAGE_SIDE as i32;
        let page = (x.div_euclid(page_side), z.div_euclid(page_side));
        let count = self.page_origins.len();
        let slot = *self.pages.entry(page).or_insert_with(|| {
            if count == 0 {
                // Reserve the call's paged allowance once, while keeping page
                // contents lazy. Dense-only calls allocate no page buffers.
                let paged_cells = self.steps.capacity() - self.flags.len();
                self.page_origins
                    .reserve_exact(paged_cells / Self::PAGE_CELLS);
                self.page_flags.reserve_exact(paged_cells);
                self.page_neighbors.reserve_exact(paged_cells);
            }
            self.page_origins
                .push((page.0 * page_side, page.1 * page_side));
            self.page_flags.resize((count + 1) * Self::PAGE_CELLS, None);
            self.page_neighbors
                .resize((count + 1) * Self::PAGE_CELLS, [0; 8]);
            self.steps.resize(
                self.flags.len() + (count + 1) * Self::PAGE_CELLS,
                Self::PAGED,
            );
            count
        });
        self.flags.len()
            + slot * Self::PAGE_CELLS
            + x.rem_euclid(page_side) as usize * Self::PAGE_SIDE
            + z.rem_euclid(page_side) as usize
    }

    #[inline]
    fn neighbor(&self, index: usize, bit: usize) -> usize {
        if self.steps[index] & Self::PAGED == 0 {
            let (dx, dz) = DIRS[bit];
            return index.wrapping_add_signed(dx as isize * self.side as isize + dz as isize);
        }
        if index >= self.flags.len() {
            return self.page_neighbors[index - self.flags.len()][bit];
        }
        self.boundary_neighbors[self.boundary_slot(index)][bit]
    }

    fn boundary_slot(&self, index: usize) -> usize {
        let (x, z) = (index / self.side, index % self.side);
        if x == 0 {
            z
        } else if x + 1 == self.side {
            self.side + z
        } else if z == 0 {
            self.side * 2 + x
        } else {
            self.side * 3 + x
        }
    }

    fn flag(&mut self, index: usize) -> Option<i32> {
        if index < self.flags.len() {
            return self.flags[index];
        }
        if self.steps[index] & Self::FLAGS_LOADED == 0 {
            let (x, z) = self.coordinates(index);
            self.page_flags[index - self.flags.len()] = self
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
            self.steps[index] |= Self::FLAGS_LOADED;
        }
        self.page_flags[index - self.flags.len()]
    }

    #[inline]
    fn forward_mask(&mut self, index: usize) -> u8 {
        if self.prepared(index) {
            return self.steps[index] as u8;
        }
        self.prepare_forward_mask(index)
    }

    fn prepare_forward_mask(&mut self, index: usize) -> u8 {
        if index < self.flags.len() {
            if let Some(mask) = self.mask(index) {
                return mask;
            }
        }
        self.steps[index] |= Self::PAGED;
        let mut mask = 0;
        let source_flags = self.flag(index);
        if source_flags.is_some() {
            let (x, z) = self.coordinates(index);
            let neighbors = DIRS.map(|(dx, dz)| self.index(x + dx, z + dz));
            if index < self.flags.len() {
                let slot = self.boundary_slot(index);
                self.boundary_neighbors[slot] = neighbors;
            } else {
                self.page_neighbors[index - self.flags.len()] = neighbors;
            }
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
        self.steps[index] |= u16::from(mask)
            | Self::PREPARED
            | if flags_open(source_flags, CollisionFlag::SQ_BLOCKED) {
                Self::WALKABLE
            } else {
                0
            };
        mask
    }

    #[inline]
    fn prepared(&self, index: usize) -> bool {
        self.steps[index] & Self::PREPARED != 0
    }

    #[inline]
    fn walkable(&self, index: usize) -> bool {
        if self.prepared(index) {
            self.steps[index] & Self::WALKABLE != 0
        } else {
            flags_open(self.flags[index], CollisionFlag::SQ_BLOCKED)
        }
    }

    #[inline]
    fn mask(&mut self, index: usize) -> Option<u8> {
        let step = self.steps[index];
        if step & Self::PAGED != 0 {
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
            // The same destination masks and orthogonal legs as can_step_local,
            // reading each orthogonal neighbor once for all eight directions.
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
fn arrival_distances<const BOXED: bool>(
    cache: &ArrivalStepCache<'_>,
    seeds: &[usize],
) -> (Vec<u32>, u32, Vec<bool>, Vec<u32>) {
    let mut depth = vec![u32::MAX; cache.flags.len()];
    let mut queue = Vec::with_capacity(seeds.len());
    let mut connected = if BOXED {
        Vec::new()
    } else {
        vec![false; cache.flags.len()]
    };
    let mut extra = Vec::new();
    for &seed in seeds {
        depth[seed] = 0;
        queue.push(seed);
        if !BOXED {
            connected[seed] = true;
        }
    }
    let flags = |x: i32, z: i32| cache.flags[x as usize * cache.side + z as usize];
    let inverse = [1, 0, 3, 2, 7, 6, 5, 4];
    let offsets = DIRS.map(|(dx, dz)| dx as isize * cache.side as isize + dz as isize);
    let mut boundary = u32::MAX;
    let mut boundary_seeds = Vec::new();
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
                cache.steps[predecessor] & (1 << inverse[bit]) != 0
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
    }
    let outside = if BOXED || boundary == u32::MAX {
        Vec::new()
    } else {
        arrival_outside_distances(cache, &depth, &boundary_seeds)
    };
    // Reverse reach alone omits forward-only dead ends. Include their weak
    // closure before using this set as a vertex-count upper bound.
    head = queue.len();
    queue.extend(extra);
    while head < queue.len() {
        let index = queue[head];
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
                    cache.steps[index] & (1 << bit) != 0
                } else {
                    can_step_local(&flags, x, z, dx, dz)
                }) || (if cache.prepared(next) {
                    cache.steps[next] & (1 << inverse[bit]) != 0
                } else {
                    can_step_local(&flags, nx, nz, -dx, -dz)
                }))
            {
                connected[next] = true;
                queue.push(next);
            }
        }
    }
    (depth, boundary, connected, outside)
}

/// Collapse unknown outside collision to an open one-tile contour. Coordinate
/// clamping maps every real outside walk onto this contour without increasing
/// its length. Real directed edges remain intact inside the cache, including
/// paths that leave and re-enter several times.
fn arrival_outside_distances(
    cache: &ArrivalStepCache<'_>,
    depth: &[u32],
    boundary_seeds: &[usize],
) -> Vec<u32> {
    let side = cache.side;
    let cells = cache.flags.len();
    let span = side + 1;
    let contour_len = span * 4;
    let mut outside = vec![u32::MAX; cells];
    let mut contour = vec![u32::MAX; contour_len];
    let mut queue = Vec::new();
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
                    cache.steps[predecessor] & (1 << inverse[bit]) != 0
                } else {
                    can_step_local(&flags, x + dx, z + dz, -dx, -dz)
                };
                if allowed {
                    outside[predecessor] = next_depth;
                    queue.push(predecessor);
                }
            }
        }
    }
    outside
}

/// One forward landmark ball supplies a shared rank lower bound. Its tree's
/// reversible paths let nearby origins reach every vertex in the ball before
/// their earliest possible goal. Small closed components are decided outright.
struct ArrivalLandmark {
    depth: Vec<u32>,
    reversible: Vec<bool>,
    queue: Vec<usize>,
    goal_reachable: Vec<bool>,
    goal_queue: Vec<usize>,
    goal_lower: u32,
}

impl ArrivalLandmark {
    fn prepare(
        &mut self,
        root: usize,
        budget: u32,
        goals: &[usize],
        cache: &mut ArrivalStepCache<'_>,
        explore_no_goal: bool,
    ) -> (Option<u32>, u32, bool) {
        for &index in &self.queue {
            self.depth[index] = u32::MAX;
            self.reversible[index] = false;
            if let Some(reachable) = self.goal_reachable.get_mut(index) {
                *reachable = false;
            }
        }
        self.queue.clear();
        self.goal_queue.clear();
        self.queue.push(root);
        self.depth[root] = 0;
        self.reversible[root] = true;
        let inverse = [1, 0, 3, 2, 7, 6, 5, 4];
        let mut head = 0;
        let mut ball_depth = None;
        while head < self.queue.len() {
            let index = self.queue[head];
            head += 1;
            let mut mask = cache.forward_mask(index);
            self.depth.resize(cache.steps.len(), u32::MAX);
            self.reversible.resize(cache.steps.len(), false);
            while mask != 0 {
                let bit = mask.trailing_zeros() as usize;
                mask &= mask - 1;
                let next = cache.neighbor(index, bit);
                if self.depth[next] != u32::MAX {
                    continue;
                }
                self.depth[next] = self.depth[index] + 1;
                self.reversible[next] =
                    self.reversible[index] && cache.forward_mask(next) & (1 << inverse[bit]) != 0;
                self.queue.push(next);
                if self.queue.len() > budget as usize {
                    let first_depth = *ball_depth.get_or_insert(self.depth[next]);
                    if !explore_no_goal
                        || goals.iter().any(|&goal| self.depth[goal] != u32::MAX)
                        || self.queue.len() > budget as usize * 2
                    {
                        self.goal_lower = goals
                            .iter()
                            .map(|&goal| self.depth[goal])
                            .min()
                            .unwrap_or(u32::MAX)
                            .min(self.depth[next]);
                        for &extra in &self.queue[budget as usize + 1..] {
                            self.depth[extra] = u32::MAX;
                            self.reversible[extra] = false;
                        }
                        self.queue.truncate(budget as usize + 1);
                        return (Some(first_depth), u32::MAX, false);
                    }
                }
            }
        }
        // The complete closure either fits the dequeue budget or contains no
        // goal. In the former case, reverse its goal paths: a directed origin
        // need not share the root's goal reach.
        if goals.iter().any(|&goal| self.depth[goal] != u32::MAX) {
            self.goal_reachable.resize(cache.steps.len(), false);
            for &goal in goals {
                if self.depth[goal] != u32::MAX {
                    self.goal_reachable[goal] = true;
                    self.goal_queue.push(goal);
                }
            }
            head = 0;
            while head < self.goal_queue.len() {
                let index = self.goal_queue[head];
                head += 1;
                for (bit, &inverse_bit) in inverse.iter().enumerate() {
                    let predecessor = cache.neighbor(index, bit);
                    if self.depth[predecessor] != u32::MAX
                        && !self.goal_reachable[predecessor]
                        && cache.steps[predecessor] & (1 << inverse_bit) != 0
                    {
                        self.goal_reachable[predecessor] = true;
                        self.goal_queue.push(predecessor);
                    }
                }
            }
        }
        self.goal_lower = goals
            .iter()
            .map(|&goal| self.depth[goal])
            .min()
            .unwrap_or(u32::MAX);
        (ball_depth, u32::MAX, true)
    }
}

/// Incoming edges read and update these states together.
#[derive(Clone, Copy, Default)]
struct ArrivalLayerCell {
    visited: u64,
    frontier: u64,
    next: u64,
    before: u64,
    next_before: u64,
    equal: u64,
}

/// Bit-parallel forward layers for up to 64 origins. Per-origin vertex counts
/// are bit-sliced, so a shared discovery increments all its origins together.
/// A canonical shortest goal path orders its prefixes lexicographically in DIRS
/// order. Tracking paths before those prefixes also gives exact dequeue ranks
/// in the budget-crossing layer, without replaying each origin's flood.
struct ArrivalLayers {
    cells: Vec<ArrivalLayerCell>,
    touched: Vec<usize>,
    queue: Vec<usize>,
    next_queue: Vec<usize>,
}

impl ArrivalLayers {
    fn new(cells: usize, capacity: usize) -> Self {
        let mut states = Vec::with_capacity(capacity);
        states.resize(cells, ArrivalLayerCell::default());
        Self {
            cells: states,
            touched: Vec::with_capacity(cells),
            queue: Vec::with_capacity(cells),
            next_queue: Vec::with_capacity(cells),
        }
    }

    fn classify(
        &mut self,
        starts: &[usize],
        goals: &[usize],
        budget: u32,
        goal_depth: &[u32],
        canonical: u64,
        cache: &mut ArrivalStepCache<'_>,
    ) -> (u64, u64) {
        for index in self.touched.drain(..) {
            self.cells[index] = ArrivalLayerCell::default();
        }
        self.queue.clear();
        self.next_queue.clear();
        let all = u64::MAX >> (64 - starts.len());
        let mut active = all;
        let mut accepted = 0;
        let mut uncertain = 0;
        // An active origin has expanded at most budget vertices; each has
        // eight successors. Nineteen planes cover 8*(2*104+1)^2+1 discoveries.
        let count_bits = (u32::BITS - budget.leading_zeros() + 3) as usize;
        let mut storage = [0u64; 19];
        let counts = &mut storage[..count_bits];
        let mut prior_counts = [0u64; 19];
        let mut positions = [usize::MAX; 64];
        counts[0] = all;
        for (bit, &start) in starts.iter().enumerate() {
            let origin = 1 << bit;
            let cell = &mut self.cells[start];
            if cell.visited == 0 {
                self.touched.push(start);
                self.queue.push(start);
            }
            cell.visited |= origin;
            cell.frontier |= origin;
            if canonical & origin != 0 {
                cell.equal |= origin;
                positions[bit] = start;
            }
        }
        let mut previous_above = 0;
        while !self.queue.is_empty() {
            let at_goal = goals
                .iter()
                .fold(0, |found, &goal| found | self.cells[goal].visited)
                & active;
            let exact = at_goal & canonical;
            if exact != 0 {
                // Rank is zero-based: all previous layers, followed by the
                // current-layer vertices whose first shortest path precedes
                // the first shortest path to any goal.
                let mut ranks = prior_counts;
                for &index in &self.queue {
                    let mut carry = self.cells[index].before & exact;
                    for plane in &mut ranks[..count_bits] {
                        let next_carry = *plane & carry;
                        *plane ^= carry;
                        carry = next_carry;
                        if carry == 0 {
                            break;
                        }
                    }
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
            }
            self.next_queue.clear();
            for q in 0..self.queue.len() {
                let index = self.queue[q];
                let cell = &mut self.cells[index];
                let origins = cell.frontier & active;
                let before = cell.before & origins;
                let equal = cell.equal & origins;
                cell.frontier = 0;
                cell.before = 0;
                if origins == 0 {
                    continue;
                }
                let mut mask = cache.forward_mask(index);
                if cache.steps.len() > self.cells.len() {
                    self.cells
                        .resize(cache.steps.len(), ArrivalLayerCell::default());
                }
                while mask != 0 {
                    let bit = mask.trailing_zeros() as usize;
                    mask &= mask - 1;
                    let next = cache.neighbor(index, bit);
                    let cell = &mut self.cells[next];
                    let fresh = origins & !cell.visited;
                    // A second shortest incoming path may precede the goal
                    // prefix even when the first incoming path did not.
                    if before | equal != 0 {
                        let same_layer = fresh | (origins & cell.next);
                        cell.next_before |= same_layer & (before | (equal & prefix_before[bit]));
                    }
                    if fresh == 0 {
                        continue;
                    }
                    if cell.visited == 0 {
                        self.touched.push(next);
                    }
                    cell.visited |= fresh;
                    if cell.next == 0 {
                        self.next_queue.push(next);
                    }
                    cell.next |= fresh;
                }
            }
            let mut prefixes = canonical & active;
            while prefixes != 0 {
                let bit = prefixes.trailing_zeros() as usize;
                prefixes &= prefixes - 1;
                self.cells[positions[bit]].equal &= !(1 << bit);
                self.cells[next_positions[bit]].equal |= 1 << bit;
            }
            positions = next_positions;
            prior_counts[..count_bits].copy_from_slice(counts);
            // Coalesce discoveries at the vertex before incrementing counts,
            // instead of repeating the carry chain for its incoming edges.
            for &index in &self.next_queue {
                let cell = &mut self.cells[index];
                let mut carry = cell.next;
                for plane in &mut *counts {
                    let next_carry = *plane & carry;
                    *plane ^= carry;
                    carry = next_carry;
                    if carry == 0 {
                        break;
                    }
                }
                cell.frontier = std::mem::take(&mut cell.next);
                cell.before = std::mem::take(&mut cell.next_before);
            }
            std::mem::swap(&mut self.queue, &mut self.next_queue);
        }
        (accepted, uncertain)
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
fn arrival_components(cache: &ArrivalStepCache<'_>, goals: &[usize]) -> (Vec<u32>, Vec<u8>) {
    let mut components = vec![u32::MAX; cache.steps.len()];
    let mut facts = Vec::new();
    let mut queue = Vec::new();
    let offsets = DIRS.map(|(dx, dz)| dx as isize * cache.side as isize + dz as isize);
    let inverse = [1, 0, 3, 2, 7, 6, 5, 4];
    for start in 0..cache.steps.len() {
        if !cache.prepared(start)
            || components[start] != u32::MAX
            || !flags_open(cache.flags[start], CollisionFlag::SQ_BLOCKED)
        {
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
                    fact |= u8::from(cache.steps[index] & (1 << bit) != 0) << 1;
                } else if components[next] == u32::MAX
                    && flags_open(cache.flags[next], CollisionFlag::SQ_BLOCKED)
                    && (cache.steps[index] & (1 << bit) != 0
                        || cache.steps[next] & (1 << inverse[bit]) != 0)
                {
                    components[next] = component;
                    queue.push(next);
                }
            }
        }
        facts.push(fact);
    }
    (components, facts)
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
    if radius < 0 {
        candidates.clear();
        return;
    }
    let radius = radius.min(104);
    let probe = CollisionArrivalProbe::new(&collision_at);
    let Some(target_flags) = collision_at(destination) else {
        candidates.retain(|&tile| is_arrived(tile, destination, radius, || &probe));
        return;
    };
    if radius == 0 || candidates.is_empty() {
        candidates.retain(|&tile| is_arrived(tile, destination, radius, || &probe));
        return;
    }
    let extent = radius * 3 + 1;
    let (Some(base_x), Some(base_z)) = (
        destination.x.checked_sub(extent),
        destination.z.checked_sub(extent),
    ) else {
        candidates.retain(|&tile| is_arrived(tile, destination, radius, || &probe));
        return;
    };
    let side = (extent * 2 + 1) as usize;
    let cells = side * side;
    let center = extent as usize * side + extent as usize;
    let cached: Vec<_> = (0..cells)
        .map(|index| {
            base_x
                .checked_add((index / side) as i32)
                .zip(base_z.checked_add((index % side) as i32))
                .and_then(|(x, z)| {
                    collision_at(WorldTile {
                        x,
                        z,
                        level: destination.level,
                    })
                })
        })
        .collect();
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
    // A landmark can explore twice the dequeue budget and load eight
    // neighbors per discovery. Reserve that allowance for paged scratch,
    // rounded to complete pages, rather than reallocating every new page.
    // Distinct roots may grow past it; the cache remains dynamically extensible.
    let paged_cells = (budget as usize * 16 + 1).div_ceil(ArrivalStepCache::PAGE_CELLS)
        * ArrivalStepCache::PAGE_CELLS;
    let scratch_cells = cells + paged_cells;
    // Use the zeroed allocation path without eagerly touching the allowance.
    // Only the dense window is live until pages are populated.
    let mut steps = vec![0; scratch_cells];
    steps.truncate(cells);
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
        pages: std::collections::HashMap::new(),
        page_origins: Vec::new(),
        page_flags: Vec::new(),
        page_neighbors: Vec::new(),
        boundary_neighbors: vec![[0; 8]; side * 4],
    };
    let lo = (extent - radius) as usize;
    let hi = (extent + radius) as usize;
    for x in lo..=hi {
        for z in lo..=hi {
            cache.mask(x * side + z);
        }
    }
    let offsets = DIRS.map(|(dx, dz)| dx as isize * side as isize + dz as isize);
    let mut exits = Vec::new();
    for x in lo..=hi {
        for z in lo..=hi {
            if x != lo && x != hi && z != lo && z != hi {
                continue;
            }
            let index = x * side + z;
            if cache.steps[index] as u8 != 0
                && offsets.iter().enumerate().any(|(bit, &offset)| {
                    cache.steps[index] & (1 << bit) != 0
                        && !cache.prepared(index.wrapping_add_signed(offset))
                })
            {
                exits.push(index);
            }
        }
    }
    // Exit seeds are the last in-box tile, one step before leaving the box.
    let exit_depth = arrival_distances::<true>(&cache, &exits).0;
    let (components, component_facts) = arrival_components(&cache, goals);
    // Boxed certificates must see the original box exits. After they are
    // captured, prepare the remaining dense steps once for all reverse edges.
    for index in 0..cells {
        cache.mask(index);
    }
    let (depth, goal_boundary, connected, outside) = arrival_distances::<false>(&cache, goals);
    let outside_goal_bound = |index: usize| {
        if goal_boundary == u32::MAX {
            u32::MAX
        } else {
            outside[index]
        }
    };
    let goal_lower_bound = |index: usize| depth[index].min(outside_goal_bound(index));
    let stride = side + 1;
    let mut prefix = vec![0u32; stride * stride];
    for x in 0..side {
        let mut row = 0;
        for z in 0..side {
            row += u32::from(connected[x * side + z]);
            prefix[(x + 1) * stride + z + 1] = prefix[x * stride + z + 1] + row;
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
    let mut keep = vec![false; candidates.len()];
    let mut uncertain = Vec::new();
    let mut replay = Vec::new();
    for (candidate, &tile) in candidates.iter().enumerate() {
        let distance = tile
            .x
            .abs_diff(destination.x)
            .max(tile.z.abs_diff(destination.z));
        if tile.level != destination.level || distance > radius as u32 {
            continue;
        }
        if tile == destination {
            keep[candidate] = true;
            continue;
        }
        let x = (tile.x - base_x) as usize;
        let z = (tile.z - base_z) as usize;
        let index = x * side + z;
        if cached[index].is_none() {
            continue;
        }
        if !flags_open(cached[index], CollisionFlag::SQ_BLOCKED) {
            replay.push(candidate);
            continue;
        }
        let d = depth[index];
        if d == u32::MAX && (goal_boundary == u32::MAX || outside[index] == u32::MAX) {
            continue;
        }
        if d != u32::MAX {
            // At depth d all dequeued tiles fit the origin's d-square. Also,
            // outside tiles cannot get farther from the target box than
            // d - first_exit + 1. Count every walkable tile, not just the path.
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
                continue;
            }
        }
        let component = components[index];
        if component != u32::MAX && component_facts[component as usize] == 0 {
            continue;
        }
        uncertain.push((candidate, index));
    }
    uncertain.sort_unstable_by_key(|&(_, start)| {
        (components[start], start / side / 8, start % side / 8, start)
    });
    let mut group = 0;
    let mut layers: Option<ArrivalLayers> = None;
    let mut landmark: Option<ArrivalLandmark> = None;
    let mut pending = Vec::new();
    let mut nearest_root = vec![u32::MAX; candidates.len()];
    while group < uncertain.len() {
        let mut end = group + 1;
        while end < uncertain.len()
            && components[uncertain[end].1] == components[uncertain[group].1]
        {
            end += 1;
        }
        let region = &uncertain[group..end];
        if region.len() <= 8
            && region
                .iter()
                .all(|&(_, start)| depth[start] <= radius as u32 * 2)
        {
            // Nearby tiny cohorts do not repay a landmark flood. Their
            // ordered probes reuse the same collision steps and epoch marks.
            replay.extend(region.iter().map(|&(candidate, _)| candidate));
            group = end;
            continue;
        }
        let root = region
            .iter()
            .enumerate()
            .filter(|&(_, &(_, start))| goal_lower_bound(start) > radius as u32 * 2)
            .min_by_key(|&(slot, &(_, start))| {
                (
                    std::cmp::Reverse(goal_lower_bound(start)),
                    slot.abs_diff(region.len() / 2),
                )
            })
            .map(|(_, &(_, start))| start)
            .unwrap_or_else(|| {
                region
                    .iter()
                    .enumerate()
                    .min_by_key(|&(slot, &(_, start))| {
                        (depth[start], slot.abs_diff(region.len() / 2))
                    })
                    .map(|(_, &(_, start))| start)
                    .expect("nonempty component group")
            });
        let landmark = landmark.get_or_insert_with(|| {
            let capacity = cache.steps.capacity();
            let mut depth = Vec::with_capacity(capacity);
            depth.resize(cache.steps.len(), u32::MAX);
            let mut reversible = Vec::with_capacity(capacity);
            reversible.resize(cache.steps.len(), false);
            ArrivalLandmark {
                depth,
                reversible,
                queue: Vec::with_capacity(budget as usize * 2 + 1),
                goal_reachable: Vec::with_capacity(capacity),
                goal_queue: Vec::with_capacity(budget as usize * 2 + 1),
                goal_lower: 0,
            }
        });
        let bounds = landmark.prepare(root, budget, goals, &mut cache, depth[root] == u32::MAX);
        let mut roots = [root; 64];
        let mut root_count = 1;
        let mut undecided = |(candidate, start): (usize, usize),
                             (ball_depth, boundary, closed): (Option<u32>, u32, bool),
                             landmark: &ArrivalLandmark,
                             nearest: &mut u32| {
            let root_distance = landmark.depth[start];
            *nearest = (*nearest).min(root_distance);
            if landmark.queue[0] == start {
                // The landmark retained exactly the first budget+1 FIFO
                // discoveries, so its own root needs no second flood.
                keep[candidate] = goals.iter().any(|&goal| landmark.depth[goal] != u32::MAX);
                return false;
            }
            if root_distance != u32::MAX {
                if closed {
                    keep[candidate] = landmark.goal_reachable.get(start).copied().unwrap_or(false);
                    return false;
                }
                if let Some(ball_depth) = ball_depth {
                    // The root can reach the origin, so every tile dequeued by
                    // its goal layer lies in the root's (root_distance+d) ball.
                    // Layers strictly below ball_depth contain at most budget
                    // vertices; an unexpanded boundary must not hide any.
                    let upper = root_distance.saturating_add(depth[start]);
                    if upper < ball_depth && upper <= boundary {
                        keep[candidate] = true;
                        return false;
                    }
                    if landmark.reversible[start]
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
        }
        if let Some(&(_, near)) = pending.iter().min_by_key(|&&(_, start)| depth[start]) {
            if near != root {
                let bounds =
                    landmark.prepare(near, budget, goals, &mut cache, depth[near] == u32::MAX);
                roots[root_count] = near;
                root_count += 1;
                pending.retain(|&entry| {
                    undecided(entry, bounds, landmark, &mut nearest_root[entry.0])
                });
            }
        }
        // Continue the shared balls from widely separated undecided origins.
        // A single component may wind far beyond the initial collision window.
        while pending.len() > 64 && root_count < roots.len() {
            let Some(&(_, root)) = pending
                .iter()
                .filter(|&&(_, start)| !roots[..root_count].contains(&start))
                .max_by_key(|&&(candidate, _)| nearest_root[candidate])
            else {
                break;
            };
            roots[root_count] = root;
            root_count += 1;
            let bounds = landmark.prepare(root, budget, goals, &mut cache, depth[root] == u32::MAX);
            pending.retain(|&entry| undecided(entry, bounds, landmark, &mut nearest_root[entry.0]));
        }
        if pending.len() <= 64 {
            // Small cohorts do not repay allocating the shared layer arrays.
            replay.extend(pending.iter().map(|&(candidate, _)| candidate));
        } else {
            for chunk in pending.chunks(64) {
                let mut starts = [0; 64];
                let mut canonical = 0;
                for (bit, &(_, start)) in chunk.iter().enumerate() {
                    starts[bit] = start;
                    // No outside path may be shorter OR tied: equal-length
                    // paths could change the canonical DIRS ordering.
                    if depth[start] < outside_goal_bound(start) {
                        canonical |= 1 << bit;
                    }
                }
                let (accepted, undecided) = layers
                    .get_or_insert_with(|| {
                        ArrivalLayers::new(cache.steps.len(), cache.steps.capacity())
                    })
                    .classify(
                        &starts[..chunk.len()],
                        goals,
                        budget,
                        &depth,
                        canonical,
                        &mut cache,
                    );
                for (bit, &(candidate, _)) in chunk.iter().enumerate() {
                    keep[candidate] = accepted & (1 << bit) != 0;
                    if undecided & (1 << bit) != 0 {
                        replay.push(candidate);
                    }
                }
            }
        }
        group = end;
    }
    let scratch = WindowReachScratch {
        marks: if replay.is_empty() {
            Vec::new()
        } else {
            Vec::with_capacity(cache.steps.capacity())
        },
        queue: if replay.is_empty() {
            Vec::new()
        } else {
            Vec::with_capacity(budget as usize * 8 + 1)
        },
        epoch: 0,
    };
    let window_probe = WindowArrivalProbe {
        fallback: &probe,
        cache: std::cell::RefCell::new(cache),
        goals,
        base: WorldTile {
            x: base_x,
            z: base_z,
            level: destination.level,
        },
        side,
        scratch: std::cell::RefCell::new(scratch),
    };
    for candidate in replay {
        keep[candidate] = is_arrived(candidates[candidate], destination, radius, || &window_probe);
    }
    let mut index = 0;
    candidates.retain(|_| {
        let result = keep[index];
        index += 1;
        result
    });
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
