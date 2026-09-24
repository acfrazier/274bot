//! Whole-world per-level collision bake: every mapsquare's MAP f-flags and
//! LOC placements → one [`WorldCollision`] of `client::dash3d::CollisionFlag`
//! bitmasks, mirroring the client's `CollisionMap` stamping, plus the
//! derived per-tile walkable word ([`derive_walkable`]) the router's
//! directional step test reads. The bake holds four planes (levels 0..=3)
//! like the client's `collision[4]`; each loc/MAP row stamps its own
//! plane, never a level-0 squash.

use std::collections::HashSet;
use std::path::Path;

use api::obj_names::LocDefs;
use api::snapshot::WorldTile;
use client::dash3d::{CollisionFlag, LocAngle, LocShape};

use crate::pack::{parse_loc_fields, parse_map_line, section, PackError};

/// Mapsquare edge length in tiles.
const SQUARE: usize = 64;

/// The number of collision planes the bake keeps (levels 0..=3), like the
/// client's `collision[4]`.
const LEVELS: usize = 4;

/// Flags that make a tile unwalkable: any wall direction, a scenery
/// footprint, or MAP-blocked ground.
const WALK_BLOCK: u32 = CollisionFlag::WALK_BLOCK_FLAGS as u32
    | CollisionFlag::WALK_SCENERY as u32
    | CollisionFlag::WR_GRND as u32;

/// The client's `SQ_BLOCKED` base shared by every `PL_WALK_*` movement mask
/// (`WALK_SCENERY | BLOCK_NPCS_AND_PLAYERS | WR_GRND` = `0x280100`): any of
/// these on a tile makes the derived walkable word reject every direction.
/// `pub(crate)` for the paint's walk-word blocked test.
pub(crate) const SQ_BLOCKED: u32 = CollisionFlag::WALK_SCENERY as u32
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

/// Bake the whole world: every `m<x>_<z>.jm2` under `maps_dir` (other files
/// are metadata, e.g. `ignore.csv`/`free2play.csv`, and are skipped). MAP
/// flags with bit 0 set stamp `WR_GRND` on their LINK_BELOW-corrected
/// plane (the client's `finishBuild`); LOC placements stamp flags by
/// shape/angle exactly like the client's `CollisionMap` (`add_wall` for
/// walls, `add_loc` footprints for scenery — roofs included — and
/// `block_ground` for active ground decor), each on its own
/// LINK_BELOW-corrected plane (the client's `loadLocations` drops a loc
/// whose `mapl[1]` tile carries LINK_BELOW below the grid and shifts a
/// level-1 loc down to level 0 — the Lumbridge castle battlements stamp
/// this way). Openable doors (`door_ids`) are stamped blocked-when-closed.
/// All squares merge into one 4-plane bounding grid (levels 0..=3).
///
/// Any `.jm2` that cannot be read, has no MAP section, or has an
/// unrecognizable mapsquare name is an error: the whole world bakes or none
/// of it does.
pub fn bake_from_maps(
    maps_dir: &Path,
    loc_defs: &LocDefs,
    door_ids: &HashSet<i32>,
) -> Result<WorldCollision, PackError> {
    // Every mapsquare file, with its square coordinates.
    let mut squares: Vec<(i32, i32)> = Vec::new();
    for ent in std::fs::read_dir(maps_dir).map_err(PackError::Io)? {
        let ent = ent.map_err(PackError::Io)?;
        let path = ent.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jm2") {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let coords = name
            .strip_prefix('m')
            .and_then(|n| n.split_once('_'))
            .and_then(|(x, z)| Some((x.parse::<i32>().ok()?, z.parse::<i32>().ok()?)))
            .ok_or_else(|| {
                PackError::BadLength(format!("{}: not an m<x>_<z> mapsquare", path.display()))
            })?;
        squares.push(coords);
    }
    if squares.is_empty() {
        return Err(PackError::BadLength(
            "no m<x>_<z>.jm2 mapsquares found".into(),
        ));
    }
    squares.sort_unstable();

    // Bounding box in tiles (the existing merge_squares geometry).
    let min_x = squares
        .iter()
        .map(|(x, _)| x * SQUARE as i32)
        .min()
        .unwrap();
    let min_z = squares
        .iter()
        .map(|(_, z)| z * SQUARE as i32)
        .min()
        .unwrap();
    let max_x = squares
        .iter()
        .map(|(x, _)| (x + 1) * SQUARE as i32)
        .max()
        .unwrap();
    let max_z = squares
        .iter()
        .map(|(_, z)| (z + 1) * SQUARE as i32)
        .max()
        .unwrap();
    let (width, height) = ((max_x - min_x) as usize, (max_z - min_z) as usize);
    let plane_cells = width * height;
    let mut flags = vec![0u32; LEVELS * plane_cells];

    for (square_x, square_z) in &squares {
        let path = maps_dir.join(format!("m{square_x}_{square_z}.jm2"));
        let text = std::fs::read_to_string(&path).map_err(PackError::Io)?;
        stamp_square(
            &mut flags, width, height, min_x, min_z, *square_x, *square_z, &text, loc_defs,
            door_ids,
        )?;
    }

    // Tiles no mapsquare covers (bbox gaps, non-rectangular world) are
    // blocked ground. Level 0 is the ground plane and is always blocked at
    // the gaps; an upper plane is only blocked at the gaps when the bake
    // actually stamped it — an unused plane stays empty (walkable) so gap
    // fill never fabricates it into an "existing" level.
    let mut covered = vec![false; plane_cells];
    for (square_x, square_z) in &squares {
        let (ox, oz) = (
            (square_x * SQUARE as i32 - min_x) as usize,
            (square_z * SQUARE as i32 - min_z) as usize,
        );
        for z in 0..SQUARE {
            for x in 0..SQUARE {
                covered[(oz + z) * width + (ox + x)] = true;
            }
        }
    }
    for level in 0..LEVELS {
        let base = level * plane_cells;
        if level > 0 && !flags[base..base + plane_cells].iter().any(|&f| f != 0) {
            continue;
        }
        for (i, c) in covered.iter().enumerate() {
            if !c {
                flags[base + i] |= CollisionFlag::WR_GRND as u32;
            }
        }
    }

    let (walk, blocked) = pack_walk(&flags);
    Ok(WorldCollision {
        origin: WorldTile {
            x: min_x,
            z: min_z,
            level: 0,
        },
        width,
        height,
        walk,
        blocked,
        flags: Some(flags),
    })
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

#[allow(clippy::too_many_arguments)]
/// Stamp one mapsquare's MAP flags and LOC placements into the bbox grid.
fn stamp_square(
    flags: &mut [u32],
    width: usize,
    height: usize,
    min_x: i32,
    min_z: i32,
    square_x: i32,
    square_z: i32,
    text: &str,
    loc_defs: &LocDefs,
    door_ids: &HashSet<i32>,
) -> Result<(), PackError> {
    let mut in_map = false;
    let mut in_loc = false;
    let mut saw_map = false;
    let mut map_rows: Vec<(i32, usize, usize, u32)> = Vec::new();
    let mut locs = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = section(line) {
            saw_map |= name == "MAP";
            in_map = name == "MAP";
            in_loc = name == "LOC";
            continue;
        }
        if in_map {
            if let Some(row) = parse_map_line(line) {
                map_rows.push(row);
            }
        } else if in_loc {
            if let Some(loc) = parse_loc_fields(line) {
                locs.push(loc);
            }
        }
    }
    if !saw_map {
        return Err(PackError::BadLength(format!(
            "m{square_x}_{square_z}.jm2: no MAP section"
        )));
    }

    // The client's `finishBuild` reads the LINK_BELOW bit from the level-1
    // map flags (`mapl[1]`) for every level's BLOCK tiles, so the MAP pass
    // is two-stage: collect the rows, then stamp with the corrected level.
    let link_below: HashSet<(usize, usize)> = map_rows
        .iter()
        .filter(|&&(level, _, _, f)| level == 1 && f & 2 != 0)
        .map(|&(_, x, z, _)| (x, z))
        .collect();
    for &(level, x, z, f) in &map_rows {
        if f & 1 == 0 {
            continue;
        }
        // Client `finishBuild` (TS 76-90): a BLOCK tile blocks its
        // LINK_BELOW-corrected plane; `true_level < 0` is never stamped.
        let true_level = if link_below.contains(&(x, z)) {
            level - 1
        } else {
            level
        };
        if true_level >= 0 {
            set_at(
                flags,
                width,
                height,
                square_x * SQUARE as i32 + x as i32 - min_x,
                square_z * SQUARE as i32 + z as i32 - min_z,
                true_level,
                CollisionFlag::WR_GRND as u32,
            );
        }
    }

    for loc in &locs {
        // Client `loadLocations` (ClientBuild.ts): a loc whose tile has
        // LINK_BELOW on the level-1 map flags is placed on `level - 1`;
        // `level - 1 < 0` drops the loc entirely (no collision plane).
        let true_level = if link_below.contains(&(loc.x, loc.z)) {
            loc.level - 1
        } else {
            loc.level
        };
        if true_level < 0 {
            continue;
        }
        // Local (origin-relative) tile coords; wall stamps may reach into
        // neighbouring squares of the same bbox.
        let (lx, lz) = (
            square_x * SQUARE as i32 + loc.x as i32 - min_x,
            square_z * SQUARE as i32 + loc.z as i32 - min_z,
        );
        let def = loc_defs.loc(loc.loc_id);
        // Unknown loc ids default to blocking, as the walk-byte bake did.
        let blockwalk = def.is_none_or(|d| d.block_walk);
        let blockrange = def.is_none_or(|d| d.block_range);
        // Openable wall doors are stamped blocked-when-closed, even though
        // the door-edge extraction is the transport task's. Everything
        // stamps its own (LINK_BELOW-corrected) level plane, never a
        // level-0 squash.
        if door_ids.contains(&loc.loc_id) && loc.shape == LocShape::WALL_STRAIGHT {
            add_wall(
                flags, width, height, lx, lz, loc.shape, loc.angle, blockrange, true_level,
            );
        } else {
            // The client `addLoc` collision table (build.rs): ground decor
            // uses `block_ground`, centrepieces/roofs/wall-diagonal use
            // `add_loc`, walls use `add_wall`, wall decor carries nothing.
            match loc.shape {
                LocShape::GROUND_DECOR => {
                    // The client's `block_ground` (WR_GRND) is what blocks
                    // walk on a blockwalk, active ground decor.
                    if blockwalk && def.is_some_and(|d| d.active) {
                        set_at(
                            flags,
                            width,
                            height,
                            lx,
                            lz,
                            true_level,
                            CollisionFlag::WR_GRND as u32,
                        );
                    }
                }
                LocShape::CENTREPIECE_STRAIGHT
                | LocShape::CENTREPIECE_DIAGONAL
                | LocShape::WALL_DIAGONAL
                | LocShape::ROOF_STRAIGHT..=LocShape::ROOFEDGE_SQUARE_CORNER => {
                    if blockwalk {
                        let (w, l) = def.map_or((1, 1), |d| (d.width, d.length));
                        add_loc(
                            flags, width, height, lx, lz, w, l, loc.angle, blockrange, true_level,
                        );
                    }
                }
                LocShape::WALL_STRAIGHT
                | LocShape::WALL_DIAGONAL_CORNER
                | LocShape::WALL_L
                | LocShape::WALL_SQUARE_CORNER
                    if blockwalk =>
                {
                    add_wall(
                        flags, width, height, lx, lz, loc.shape, loc.angle, blockrange, true_level,
                    );
                }
                // Wall decor (4..=8) and unknown shapes carry no collision.
                _ => {}
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
/// Client `CollisionMap.add_wall` mirror: stamp wall direction flags on the
/// wall tile and the tiles it borders, exactly like the client.
fn add_wall(
    flags: &mut [u32],
    width: usize,
    height: usize,
    x: i32,
    z: i32,
    shape: i32,
    angle: i32,
    blockrange: bool,
    level: i32,
) {
    let (west, east, north, south) = if blockrange {
        (
            CollisionFlag::V_W as u32,
            CollisionFlag::V_E as u32,
            CollisionFlag::V_N as u32,
            CollisionFlag::V_S as u32,
        )
    } else {
        (
            CollisionFlag::W_W as u32,
            CollisionFlag::W_E as u32,
            CollisionFlag::W_N as u32,
            CollisionFlag::W_S as u32,
        )
    };
    let (north_west, south_east, north_east, south_west) = if blockrange {
        (
            CollisionFlag::V_NW as u32,
            CollisionFlag::V_SE as u32,
            CollisionFlag::V_NE as u32,
            CollisionFlag::V_SW as u32,
        )
    } else {
        (
            CollisionFlag::W_NW as u32,
            CollisionFlag::W_SE as u32,
            CollisionFlag::W_NE as u32,
            CollisionFlag::W_SW as u32,
        )
    };
    if shape == LocShape::WALL_STRAIGHT {
        if angle == LocAngle::WEST {
            set_at(flags, width, height, x, z, level, west);
            set_at(flags, width, height, x - 1, z, level, east);
        } else if angle == LocAngle::NORTH {
            set_at(flags, width, height, x, z, level, north);
            set_at(flags, width, height, x, z + 1, level, south);
        } else if angle == LocAngle::EAST {
            set_at(flags, width, height, x, z, level, east);
            set_at(flags, width, height, x + 1, z, level, west);
        } else if angle == LocAngle::SOUTH {
            set_at(flags, width, height, x, z, level, south);
            set_at(flags, width, height, x, z - 1, level, north);
        }
    } else if shape == LocShape::WALL_DIAGONAL_CORNER || shape == LocShape::WALL_SQUARE_CORNER {
        if angle == LocAngle::WEST {
            set_at(flags, width, height, x, z, level, north_west);
            set_at(flags, width, height, x - 1, z + 1, level, south_east);
        } else if angle == LocAngle::NORTH {
            set_at(flags, width, height, x, z, level, north_east);
            set_at(flags, width, height, x + 1, z + 1, level, south_west);
        } else if angle == LocAngle::EAST {
            set_at(flags, width, height, x, z, level, south_east);
            set_at(flags, width, height, x + 1, z - 1, level, north_west);
        } else if angle == LocAngle::SOUTH {
            set_at(flags, width, height, x, z, level, south_west);
            set_at(flags, width, height, x - 1, z - 1, level, north_east);
        }
    } else if shape == LocShape::WALL_L {
        if angle == LocAngle::WEST {
            set_at(flags, width, height, x, z, level, north | west);
            set_at(flags, width, height, x - 1, z, level, east);
            set_at(flags, width, height, x, z + 1, level, south);
        } else if angle == LocAngle::NORTH {
            set_at(flags, width, height, x, z, level, north | east);
            set_at(flags, width, height, x, z + 1, level, south);
            set_at(flags, width, height, x + 1, z, level, west);
        } else if angle == LocAngle::EAST {
            set_at(flags, width, height, x, z, level, south | east);
            set_at(flags, width, height, x + 1, z, level, west);
            set_at(flags, width, height, x, z - 1, level, north);
        } else if angle == LocAngle::SOUTH {
            set_at(flags, width, height, x, z, level, south | west);
            set_at(flags, width, height, x, z - 1, level, north);
            set_at(flags, width, height, x - 1, z, level, east);
        }
    }
    if blockrange {
        add_wall(flags, width, height, x, z, shape, angle, false, level);
    }
}

#[allow(clippy::too_many_arguments)]
/// Client `CollisionMap.add_loc` mirror: stamp a `WALK_SCENERY` footprint,
/// swapping width/length for north/south angles.
fn add_loc(
    flags: &mut [u32],
    width: usize,
    height: usize,
    x: i32,
    z: i32,
    size_x: i32,
    size_z: i32,
    angle: i32,
    blockrange: bool,
    level: i32,
) {
    let mut f = CollisionFlag::WALK_SCENERY as u32;
    if blockrange {
        f |= CollisionFlag::VIS_SCENERY as u32;
    }
    let (sx, sz) = if angle == LocAngle::NORTH || angle == LocAngle::SOUTH {
        (size_z, size_x)
    } else {
        (size_x, size_z)
    };
    for tx in x..x + sx {
        for tz in z..z + sz {
            set_at(flags, width, height, tx, tz, level, f);
        }
    }
}

/// OR `f` into the tile at local grid coords `(x, z, level)`, dropping
/// tiles outside the bbox or on unknown levels (the client drops region-
/// edge stamps the same way).
fn set_at(flags: &mut [u32], width: usize, height: usize, x: i32, z: i32, level: i32, f: u32) {
    if level < 0 || level >= LEVELS as i32 {
        return;
    }
    if x < 0 || z < 0 {
        return;
    }
    let (x, z) = (x as usize, z as usize);
    if x >= width || z >= height {
        return;
    }
    flags[level as usize * (width * height) + z * width + x] |= f;
}

#[cfg(test)]
#[path = "collision_tests.rs"]
mod tests;
