//! Runtime whole-world per-level collision model and shared movement helpers.
//! Source MAP/LOC stamping lives in `nav-bake`; this module owns collision
//! queries, plane conversion, and compact walk-word helpers shared by the
//! router, paints, map views, and baker.

use api::snapshot::WorldTile;
use client::dash3d::CollisionFlag;

/// Number of collision planes represented by [`WorldCollision`] (levels 0..=3),
/// matching the client's `collision[4]`. Shared with `nav-bake` so its
/// source grids use the runtime layout.
pub const LEVELS: usize = 4;

/// Convert a raw visual MAP/LOC plane to the client's collision/game plane.
/// `link_below` is bit 0x2 of **plane 1** at this x/z, regardless of the
/// source plane. A shifted plane zero has no game-plane placement.
/// Already-game-plane coordinates (NPC spawns, routes, live tiles) must not
/// call this helper. Invalid raw planes are absent, not clamped.
pub fn game_plane(raw_plane: i32, link_below: bool) -> Option<i32> {
    if !(0..LEVELS as i32).contains(&raw_plane) {
        return None;
    }
    let plane = raw_plane - i32::from(link_below);
    (plane >= 0).then_some(plane)
}

/// Flags that make a tile unwalkable: any wall direction, a scenery
/// footprint, or MAP-blocked ground.
const WALK_BLOCK: u32 = CollisionFlag::WALK_BLOCK_FLAGS as u32
    | CollisionFlag::WALK_SCENERY as u32
    | CollisionFlag::WR_GRND as u32;

/// The client's `SQ_BLOCKED` base shared by every `PL_WALK_*` movement mask
/// (`WALK_SCENERY | BLOCK_NPCS_AND_PLAYERS | WR_GRND` = `0x280100`): any of
/// these on a tile makes the derived walkable word reject every direction.
/// Shared by the paint's walk-word blocked test and `pack_walk`'s packed bit.
pub const SQ_BLOCKED: u32 = CollisionFlag::WALK_SCENERY as u32
    | CollisionFlag::BLOCK_NPCS_AND_PLAYERS as u32
    | CollisionFlag::WR_GRND as u32;

/// Raw walk wall bit → the client's directional `PL_WALK_*` movement mask.
const WALK_BITS: [(u32, u32); 8] = [
    (CollisionFlag::W_N as u32, CollisionFlag::PL_WALK_N as u32),
    (CollisionFlag::W_E as u32, CollisionFlag::PL_WALK_E as u32),
    (CollisionFlag::W_S as u32, CollisionFlag::PL_WALK_S as u32),
    (CollisionFlag::W_W as u32, CollisionFlag::PL_WALK_W as u32),
    (CollisionFlag::W_NE as u32, CollisionFlag::PL_WALK_NE as u32),
    (CollisionFlag::W_SE as u32, CollisionFlag::PL_WALK_SE as u32),
    (CollisionFlag::W_NW as u32, CollisionFlag::PL_WALK_NW as u32),
    (CollisionFlag::W_SW as u32, CollisionFlag::PL_WALK_SW as u32),
];

/// Whole-world per-level collision: the compact packed walk surface per
/// tile per level, four planes (levels 0..=3) like the client's
/// `collision[4]`. Both buffers are level-major: index
/// `level * width * height + z * width + x`, row-major `z` then `x`
/// within each plane. A plane is only as dense as the maps stamped it;
/// levels with no content are empty (walkable, never a level-0 reuse).
pub struct WorldCollision {
    /// The tile at `walk[0]`; the grid spans `width` tiles in +x then
    /// `height` rows in +z, replicated across all four level planes.
    pub origin: WorldTile,
    pub width: usize,
    pub height: usize,
    /// The packed walk surface: the raw `W_*` face flags per tile per
    /// level (the old u16 walk word's bits 0-7), one byte per tile.
    /// Always resident — the compact v7 wire form — and the router's
    /// `step_ok` reads it, never the raw `flags`.
    pub walk: Vec<u8>,
    /// One `SQ_BLOCKED` bit per tile per level (the old u16 walk word's
    /// bit 8: any `WALK_SCENERY`/`BLOCK_NPCS_AND_PLAYERS`/`WR_GRND`
    /// constituent set), packed 64 cells per u64 word with the same
    /// level-major indexing as `walk`. [`walk_word_from_parts`]
    /// reconstructs the full derived word.
    pub blocked: Vec<u64>,
    /// The raw baked flags per tile per level (the client's `W_*`/`V_*`
    /// wall bits, `WALK_SCENERY` footprints, and `WR_GRND` ground blocks,
    /// exactly as `CollisionMap.add_wall`/`add_loc`/`block_ground` stamp
    /// them), only while the flags sidecar is loaded for debug paints.
    /// `None` when the sidecar is not mapped: [`Self::flag`] reads 0 and
    /// the paint faces come from the reconstructed walk word.
    pub flags: Option<Vec<u32>>,
}

impl WorldCollision {
    /// The plane index for `level` (0..=3), `None` for unknown levels.
    fn plane(&self, level: i32) -> Option<usize> {
        (0..LEVELS as i32)
            .contains(&level)
            .then_some(level as usize)
    }

    /// Tiles in one level plane (`width × height`).
    fn plane_cells(&self) -> usize {
        self.width * self.height
    }

    /// Whether the packed `SQ_BLOCKED` bit of the cell at `idx` is set.
    fn cell_blocked(&self, idx: usize) -> bool {
        (self.blocked[idx >> 6] >> (idx & 63)) & 1 != 0
    }

    /// The collision bitmask at `(x, z, level)`, `0` for tiles outside the
    /// grid, unknown levels (an empty plane is never a level-0 reuse), and
    /// for every tile while the flags sidecar is not loaded (`flags` is
    /// `None` — the paint then reads the walk word, see
    /// [`crate::paint::collision_at`]).
    pub fn flag(&self, x: i32, z: i32, level: i32) -> u32 {
        match &self.flags {
            Some(flags) => self.flag_index(flags, x, z, level),
            None => 0,
        }
    }

    /// Index a raw flags buffer with the same plane/bounds rule as
    /// [`Self::flag`] — the collision paint's sidecar path indexes a
    /// panel-held side table this way while the shared world's `flags` is
    /// `None` (see [`crate::paint::collision_at_with`]). The buffer must
    /// match the grid header (`origin`/`width`/`height`).
    pub(crate) fn flag_index(&self, flags: &[u32], x: i32, z: i32, level: i32) -> u32 {
        let Some(plane) = self.plane(level) else {
            return 0;
        };
        let lx = x - self.origin.x;
        let lz = z - self.origin.z;
        if lx < 0 || lz < 0 {
            return 0;
        }
        let (lx, lz) = (lx as usize, lz as usize);
        if lx >= self.width || lz >= self.height {
            return 0;
        }
        flags[plane * self.plane_cells() + lz * self.width + lx]
    }

    /// Load the raw baked flags (a `decode_flags_sidecar` buffer) into the
    /// collision for debug paints: [`Self::flag`] then answers from the raw
    /// flags instead of the walk reconstruction. Takes `&mut self` — the
    /// exclusive-owner attach path; a shared `Arc` world maps the sidecar
    /// through the panel's side table instead (see
    /// [`crate::paint::collision_at_with`]).
    pub fn attach_flags(&mut self, flags: Vec<u32>) {
        self.flags = Some(flags);
    }

    /// Drop the loaded flags sidecar (both collision toggles off):
    /// [`Self::flag`] reads 0 again and the paint falls back to the walk
    /// word. The walk grid is untouched.
    pub fn drop_flags(&mut self) {
        self.flags = None;
    }

    /// The derived directional walkable word at `(x, z, level)`, `0` for
    /// tiles outside the grid and for unknown levels (same indexing as
    /// [`Self::flag`]). Reconstructed from the packed walk word, so it is
    /// identical whether or not the flags sidecar is loaded.
    pub fn walkable_word(&self, x: i32, z: i32, level: i32) -> u32 {
        let Some(plane) = self.plane(level) else {
            return 0;
        };
        let lx = x - self.origin.x;
        let lz = z - self.origin.z;
        if lx < 0 || lz < 0 {
            return 0;
        }
        let (lx, lz) = (lx as usize, lz as usize);
        if lx >= self.width || lz >= self.height {
            return 0;
        }
        let idx = plane * self.plane_cells() + lz * self.width + lx;
        walk_word_from_parts(self.walk[idx], self.cell_blocked(idx))
    }

    /// True when `t` sits on a baked level plane (0..=3), inside the grid
    /// bounds, and has no walk-blocking flag on that plane. Tiles outside
    /// the grid or on unknown levels are not walkable (the grid covers the
    /// whole world; beyond it is not a map). An empty plane has no blocks,
    /// so its tiles are walkable. Without the flags sidecar the walk word
    /// answers: any face bit or the `SQ_BLOCKED` bit rejects the tile.
    pub fn walkable(&self, t: WorldTile) -> bool {
        let Some(plane) = self.plane(t.level) else {
            return false;
        };
        let lx = t.x - self.origin.x;
        let lz = t.z - self.origin.z;
        if lx < 0 || lz < 0 {
            return false;
        }
        let (lx, lz) = (lx as usize, lz as usize);
        if lx >= self.width || lz >= self.height {
            return false;
        }
        let idx = plane * self.plane_cells() + lz * self.width + lx;
        match &self.flags {
            Some(flags) => flags[idx] & WALK_BLOCK == 0,
            None => walk_word_from_parts(self.walk[idx], self.cell_blocked(idx)) & WALK_BLOCK == 0,
        }
    }

    /// True when `t` sits on a baked level plane, inside the grid bounds,
    /// and has no footprint block on that plane: no `WALK_SCENERY`
    /// footprint, no `WR_GRND` ground block, and no `SQ_BLOCKED` base.
    /// Directional face flags (`W_N`/`W_S`/`V_*`/…) do NOT disqualify
    /// standing — a wall's face-flagged floor tile can be stood on even
    /// though the router can never walk onto it. The transport
    /// interact-target neighbourhood is tested against this, never the
    /// stricter [`Self::walkable`].
    pub fn standable(&self, t: WorldTile) -> bool {
        let Some(plane) = self.plane(t.level) else {
            return false;
        };
        let lx = t.x - self.origin.x;
        let lz = t.z - self.origin.z;
        if lx < 0 || lz < 0 {
            return false;
        }
        let (lx, lz) = (lx as usize, lz as usize);
        if lx >= self.width || lz >= self.height {
            return false;
        }
        let idx = plane * self.plane_cells() + lz * self.width + lx;
        match &self.flags {
            Some(flags) => flags[idx] & SQ_BLOCKED == 0,
            // The packed blocked bit is exactly the SQ_BLOCKED presence.
            None => !self.cell_blocked(idx),
        }
    }

    /// The nearest [`Self::walkable`] tile at least one step from `t` along
    /// `(dx, dz)` — the door-edge snap: a door's blind ±1 `from`/`to` can
    /// land on a wall loc right outside the door (wall 980 south of the
    /// Catherby range-house door), and the router can no longer step onto
    /// that tile. A door at the edge of the bake keeps its blind ±1
    /// neighbour rather than walking off the grid. The snap walks `t`'s own
    /// plane.
    pub fn nearest_walkable(&self, t: WorldTile, dx: i32, dz: i32) -> WorldTile {
        let (mut x, mut z) = (t.x, t.z);
        loop {
            x += dx;
            z += dz;
            let inside = self.plane(t.level).is_some()
                && x >= self.origin.x
                && z >= self.origin.z
                && (x - self.origin.x) < self.width as i32
                && (z - self.origin.z) < self.height as i32;
            if !inside {
                return WorldTile {
                    x: t.x + dx,
                    z: t.z + dz,
                    level: t.level,
                };
            }
            if self.walkable(WorldTile {
                x,
                z,
                level: t.level,
            }) {
                return WorldTile {
                    x,
                    z,
                    level: t.level,
                };
            }
        }
    }
}


/// Derive the walkable word from the raw collision flags. Footprint and
/// ground (`SQ_BLOCKED`) reject every `tryMove` direction because those
/// masks share that base. Face flags (`W_N`/`W_S`/…) only reject the
/// matching face — they must not OR the full `PL_WALK_*` word, which
/// re-injects `SQ_BLOCKED` and seals open doorways (a Seers-bank stand
/// became a 31-tile pocket). The `SQ_BLOCKED` base is normalized to the
/// full mask whenever any constituent is set, so the packed walk surface
/// (which records only the presence of the base) round-trips exactly.
pub fn derive_walkable(flags: &[u32]) -> Vec<u32> {
    flags
        .iter()
        .map(|&raw| {
            let mut w = if raw & SQ_BLOCKED != 0 { SQ_BLOCKED } else { 0 };
            for &(bit, _mask) in &WALK_BITS {
                if raw & bit != 0 {
                    w |= bit;
                }
            }
            w
        })
        .collect()
}

/// Pack the raw collision flags into the compact walk surface: a `u8` of
/// the `WALK_BLOCK_FLAGS` face bits per cell plus a bit-plane recording
/// whether any `SQ_BLOCKED` constituent is set
/// ([`walk_word_from_parts`] reconstructs the full derived word). Same
/// level-major four-plane indexing as the flags; the bit-plane packs 64
/// cells per `u64` word.
pub fn pack_walk(flags: &[u32]) -> (Vec<u8>, Vec<u64>) {
    let face = flags
        .iter()
        .map(|&raw| (raw & CollisionFlag::WALK_BLOCK_FLAGS as u32) as u8)
        .collect();
    let mut blocked = vec![0u64; flags.len().div_ceil(64)];
    for (i, &raw) in flags.iter().enumerate() {
        if raw & SQ_BLOCKED != 0 {
            blocked[i >> 6] |= 1 << (i & 63);
        }
    }
    (face, blocked)
}

/// Reconstruct the derived walkable word from the packed walk surface: the
/// `W_*` face bits, plus the full `SQ_BLOCKED` base when the cell's
/// blocked bit is set.
pub fn walk_word_from_parts(face: u8, blocked: bool) -> u32 {
    let faces = face as u32;
    if blocked {
        faces | SQ_BLOCKED
    } else {
        faces
    }
}

#[cfg(test)]
#[path = "collision_tests.rs"]
mod tests;
