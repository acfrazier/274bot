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
fn can_step_local(
    flags: &dyn Fn(i32, i32) -> Option<i32>,
    lx: i32,
    lz: i32,
    dx: i32,
    dz: i32,
) -> bool {
    let nx = lx + dx;
    let nz = lz + dz;
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

/// BFS over `can_step_local` (the m8aq `canReachLocal`).
fn can_reach_local(
    flags: &dyn Fn(i32, i32) -> Option<i32>,
    from: (i32, i32),
    to: (i32, i32),
    max_steps: u32,
    adjacent_ok: bool,
) -> bool {
    if flags(from.0, from.1).is_none() {
        return false;
    }
    let key = |lx: i32, lz: i32| lx * 256 + lz;
    let mut seen: Vec<i32> = vec![key(from.0, from.1)];
    let mut queue: Vec<(i32, i32)> = vec![from];
    let mut head = 0;
    let mut expansions = 0u32;
    while head < queue.len() {
        let cur = queue[head];
        head += 1;
        if cur == to {
            return true;
        }
        if adjacent_ok
            && (cur.0 - to.0).abs() + (cur.1 - to.1).abs() == 1
            && can_reach_adjacent_tile(flags, to.0, to.1, to.0 - cur.0, to.1 - cur.1)
        {
            return true;
        }
        expansions += 1;
        if expansions > max_steps {
            return false;
        }
        for (dx, dz) in DIRS {
            let k = key(cur.0 + dx, cur.1 + dz);
            if !seen.contains(&k) && can_step_local(flags, cur.0, cur.1, dx, dz) {
                seen.push(k);
                queue.push((cur.0 + dx, cur.1 + dz));
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
        let destination_local = self.to_local(destination);
        ORTHO.into_iter().filter_map(move |(stand_dx, stand_dz)| {
            let to = destination_local?;
            let stand = WorldTile {
                x: destination.x + stand_dx,
                z: destination.z + stand_dz,
                level: destination.level,
            };
            let flags = |lx: i32, lz: i32| self.collision_at_local(LocalTile { lx, lz });
            (self.walkable(stand)
                && can_reach_adjacent_tile(&flags, to.lx, to.lz, -stand_dx, -stand_dz))
            .then_some(stand)
        })
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

/// Frozen `ARRIVAL_MAX_STEPS` (`geometry/Reachability.ts:8`): the BFS
/// budget of both arrival reach probes.
pub const ARRIVAL_MAX_STEPS: u32 = 512;

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

/// Walk arrival, the one rule the isolate walk wait and the host follow
/// share: frozen `isArrived` (`geometry/arrival.ts:18-35`) over
/// `Reachability.arrivalProbe()` (`geometry/Reachability.ts:74-81`).
///
/// Same level, then Chebyshev `<= radius`; standing on `dest` arrives.
/// Otherwise, in order: `canReach(dest)` within [`ARRIVAL_MAX_STEPS`]
/// arrives; a walkable dest that is not reached does not; an unwalkable
/// dest arrives when the scene cannot probe it, or when the bounded
/// adjacent reach (`adjacentOk`) touches it through an open wall edge.
///
/// `view` is asked only when a probe is needed (`0 < dist <= radius`), so a
/// caller whose reach view sits behind a cache or a lock pays for it only
/// then. Reach reads the view's flood ranks: O(1), no BFS. The probes run
/// from `me`, so a flood posted from another tile cannot answer them; both
/// reach probes read false until a flood from `me` is posted.
pub fn is_arrived<V: std::ops::Deref<Target = ReachQueryView>>(
    me: WorldTile,
    dest: WorldTile,
    radius: i32,
    view: impl FnOnce() -> V,
) -> bool {
    if me.level != dest.level {
        return false;
    }
    let dist = me.x.abs_diff(dest.x).max(me.z.abs_diff(dest.z));
    if !u32::try_from(radius).is_ok_and(|radius| dist <= radius) {
        return false;
    }
    if dist == 0 {
        return true;
    }
    let view = view();
    let from_me = view.flooded_from(me);
    let reach = |adjacent_ok| {
        from_me
            && view.can_reach(
                dest,
                &SceneReachOptions {
                    max_steps: Some(ARRIVAL_MAX_STEPS),
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
