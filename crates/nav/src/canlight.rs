//! Static `area_allow_loc_add` subset baked as the `274L` canlight sidecar.
//!
//! The bit means the immutable content would allow a loc add: the tile is
//! on a loaded mapsquare, outside every `bank_zones` AABB, has no baked
//! engine FLOOR / FLOOR_DECORATION / LOC analogue (`WR_GRND` /
//! `WALK_SCENERY`), and is not covered by a static **active** map-loc
//! footprint (MAP_LOCADDUNSAFE rotation). It is not walkable, not a
//! handwritten bank box, and not a complete live predicate. Live loc
//! add/del/morph, open doors, F2P members tiles, and multi-tile live
//! footprints stay residual. Static map locs are a conservative baseline:
//! a later removal can leave a tile excluded.

use std::collections::HashSet;
use std::path::Path;

use api::obj_names::LocDefs;
use api::snapshot::WorldTile;
use client::dash3d::{CollisionFlag, LocAngle};
use sha2::{Digest, Sha256};

use crate::collision::{game_plane, WorldCollision};
use crate::pack::{parse_loc_fields, parse_map_line, section, PackError};

/// Manual policy identity. Bump when the static mask rules change in a way
/// source bytes in [`crate::bake::GENERATOR_SOURCES`] should not have to
/// carry alone (the sidecar header binding includes this id).
pub const POLICY_ID: &str = "canlight-1";
/// Algorithm label mixed into the policy digest. Describes the static
/// `area_allow_loc_add` subset: bank AABBs, WR_GRND, WALK_SCENERY, static
/// active loc footprints, mapsquare coverage on every plane.
pub const ALGORITHM_ID: &str = "static-area-allow-loc-add-v1";
/// Content-root-relative bank_zones dbrow both revisions use.
pub const BANK_ZONES_REL: &str = "scripts/skill_firemaking/configs/bank_zones.dbrow";

const SQUARE: i32 = 64;
const LEVELS: i32 = 4;

/// One inclusive `inzone` AABB from `bank_zones.dbrow`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BankZone {
    pub from: WorldTile,
    pub to: WorldTile,
}

impl BankZone {
    /// Engine `INZONE`: inclusive `from..=to` on x, level, and z. Authored
    /// order is preserved; a reversed pair matches the engine (never hits).
    pub fn contains(self, tile: WorldTile) -> bool {
        tile.x >= self.from.x
            && tile.x <= self.to.x
            && tile.level >= self.from.level
            && tile.level <= self.to.level
            && tile.z >= self.from.z
            && tile.z <= self.to.z
    }
}

/// Parse `bank_zones.dbrow`. Missing pairs, unknown rows, or a malformed
/// packed coord fail closed.
pub fn parse_bank_zones(text: &str) -> Result<Vec<BankZone>, String> {
    let mut zones = Vec::new();
    let mut saw_header = false;
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        if line == "[bank_zones]" {
            saw_header = true;
            continue;
        }
        if let Some(rest) = line.strip_prefix("table=") {
            if rest.trim() != "coord_pair_table" {
                return Err(format!(
                    "bank_zones line {}: expected table=coord_pair_table, got {line}",
                    index + 1
                ));
            }
            saw_header = true;
            continue;
        }
        if let Some(rest) = line.strip_prefix("data=") {
            let rest = rest.trim();
            let Some((kind, pair)) = rest.split_once(',') else {
                return Err(format!("bank_zones line {}: malformed data row", index + 1));
            };
            if kind.trim() != "coord_pair" {
                return Err(format!(
                    "bank_zones line {}: expected data=coord_pair, got {line}",
                    index + 1
                ));
            }
            let Some((from, to)) = pair.split_once(',') else {
                return Err(format!(
                    "bank_zones line {}: coord_pair needs two packed coords",
                    index + 1
                ));
            };
            zones.push(BankZone {
                from: unpack_packed_coord(from.trim())?,
                to: unpack_packed_coord(to.trim())?,
            });
            continue;
        }
        return Err(format!("bank_zones line {}: unexpected {line}", index + 1));
    }
    if !saw_header {
        return Err("bank_zones.dbrow has no [bank_zones] / coord_pair_table header".into());
    }
    if zones.is_empty() {
        return Err("bank_zones.dbrow has no coord_pair rows".into());
    }
    Ok(zones)
}

/// Packed coord `level_mapX_mapZ_localX_localZ` → world tile
/// (`map*64 + local`).
pub fn unpack_packed_coord(token: &str) -> Result<WorldTile, String> {
    let mut parts = token.split('_');
    let mut next = |name: &str| -> Result<i32, String> {
        let part = parts.next().ok_or_else(|| {
            format!("packed coord {token:?} is not level_mapX_mapZ_localX_localZ")
        })?;
        part.parse::<i32>()
            .map_err(|_| format!("packed coord {token:?} has a non-integer {name}"))
    };
    let level = next("level")?;
    let map_x = next("mapX")?;
    let map_z = next("mapZ")?;
    let local_x = next("localX")?;
    let local_z = next("localZ")?;
    if parts.next().is_some() {
        return Err(format!(
            "packed coord {token:?} is not level_mapX_mapZ_localX_localZ"
        ));
    }
    if !(0..LEVELS).contains(&level) {
        return Err(format!(
            "packed coord {token:?} has level {level} outside 0..=3"
        ));
    }
    if !(0..SQUARE).contains(&local_x) || !(0..SQUARE).contains(&local_z) {
        return Err(format!(
            "packed coord {token:?} has a local coordinate outside 0..64"
        ));
    }
    Ok(WorldTile {
        x: map_x * SQUARE + local_x,
        z: map_z * SQUARE + local_z,
        level,
    })
}

/// Policy digest mixed into the canlight identity: policy id, algorithm id,
/// revision, and the canonical `bank_zones.dbrow` bytes. Pack SHA and mask
/// bytes are **not** included here; [`identity_digest`] adds the baked mask
/// and [`header_binding`] concatenates the pack identity.
pub fn policy_digest(revision: u16, bank_zones_bytes: &[u8]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(POLICY_ID.as_bytes());
    digest.update([0]);
    digest.update(ALGORITHM_ID.as_bytes());
    digest.update([0]);
    digest.update(revision.to_le_bytes());
    digest.update([0]);
    digest.update(bank_zones_bytes);
    digest.finalize().into()
}

/// Identity of the actual baked canonical bitmap. The words are serialized
/// explicitly as little-endian u64 values so the identity is independent of
/// host endianness and cannot collide for equal policy inputs with different
/// masks. Runtime validation uses this small digest from the identity row; it
/// never hashes or copies the resident mask.
pub fn identity_digest(revision: u16, bank_zones_bytes: &[u8], bits: &[u64]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"canlight-mask-identity-v1");
    digest.update([0]);
    digest.update(policy_digest(revision, bank_zones_bytes));
    digest.update((bits.len() as u64).to_le_bytes());
    for word in bits {
        digest.update(word.to_le_bytes());
    }
    digest.finalize().into()
}

/// 32-byte sidecar header binding: pack digest then canlight identity digest.
/// Cheap to recompute at bundled load from identity-row hex; does not hash the mask.
pub fn header_binding(pack_digest: &[u8; 32], policy: &[u8; 32]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(pack_digest);
    digest.update(policy);
    digest.finalize().into()
}

/// Recompute the expected header binding from identity-row hex. Fails closed
/// on a malformed digest rather than treating the sidecar as unbound.
pub fn expected_header_binding(
    nav_sha256_hex: &str,
    canlight_identity_hex: &str,
) -> Result<[u8; 32], String> {
    let pack =
        crate::pack::sha256_from_hex(nav_sha256_hex).map_err(|e| format!("pack identity {e}"))?;
    let policy = crate::pack::sha256_from_hex(canlight_identity_hex)
        .map_err(|e| format!("canlight identity {e}"))?;
    Ok(header_binding(&pack, &policy))
}

/// Bake the static canlight bitset for `collision`'s geometry. `flags` is
/// the raw bake still held before `flags.take()`. Loaded `.jm2` names
/// define coverage on every plane; LOC placements reuse the collision
/// parser (one extra maps-dir pass at bake, not a second content tree).
pub fn bake_canlight(
    collision: &WorldCollision,
    flags: &[u32],
    maps_dir: &Path,
    loc_defs: &LocDefs,
    zones: &[BankZone],
) -> Result<Vec<u64>, String> {
    let plane = collision.width.saturating_mul(collision.height);
    let cells = plane.saturating_mul(4);
    if flags.len() != cells {
        return Err(format!(
            "canlight flags length {} does not match collision geometry {}x{}x4",
            flags.len(),
            collision.width,
            collision.height
        ));
    }
    if zones.is_empty() {
        return Err("canlight bake requires parsed bank_zones".into());
    }
    let squares = list_mapsquares(maps_dir)?;
    let covered = coverage_plane(collision, &squares);
    let mut unsafe_locs = vec![false; cells];
    stamp_active_loc_coverage(collision, maps_dir, &squares, loc_defs, &mut unsafe_locs)?;

    let words = cells.div_ceil(64);
    let mut bits = vec![0u64; words];
    let wr_grnd = CollisionFlag::WR_GRND as u32;
    let scenery = CollisionFlag::WALK_SCENERY as u32;
    for level in 0..4i32 {
        for lz in 0..collision.height {
            for lx in 0..collision.width {
                let idx = (level as usize) * plane + lz * collision.width + lx;
                if !covered[lz * collision.width + lx] {
                    continue;
                }
                let tile = WorldTile {
                    x: collision.origin.x + lx as i32,
                    z: collision.origin.z + lz as i32,
                    level,
                };
                if zones.iter().any(|zone| zone.contains(tile)) {
                    continue;
                }
                let raw = flags[idx];
                if raw & wr_grnd != 0 || raw & scenery != 0 {
                    continue;
                }
                if unsafe_locs[idx] {
                    continue;
                }
                bits[idx / 64] |= 1 << (idx % 64);
            }
        }
    }
    Ok(bits)
}

/// Walk-buffer index of `tile`, `None` outside the grid or on unknown levels.
pub fn canlight_cell_index(collision: &WorldCollision, tile: WorldTile) -> Option<usize> {
    if !(0..LEVELS).contains(&tile.level) {
        return None;
    }
    let lx = tile.x - collision.origin.x;
    let lz = tile.z - collision.origin.z;
    if lx < 0 || lz < 0 {
        return None;
    }
    let (lx, lz) = (lx as usize, lz as usize);
    if lx >= collision.width || lz >= collision.height {
        return None;
    }
    Some(tile.level as usize * collision.width * collision.height + lz * collision.width + lx)
}

/// Whether `tile` is set in a canlight bitset.
pub fn canlight_at(bits: &[u64], collision: &WorldCollision, tile: WorldTile) -> bool {
    let Some(idx) = canlight_cell_index(collision, tile) else {
        return false;
    };
    bits.get(idx / 64)
        .is_some_and(|word| word & (1 << (idx % 64)) != 0)
}

fn list_mapsquares(maps_dir: &Path) -> Result<Vec<(i32, i32)>, String> {
    let mut squares = Vec::new();
    let entries = std::fs::read_dir(maps_dir)
        .map_err(|e| format!("canlight maps {}: {e}", maps_dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("canlight maps {}: {e}", maps_dir.display()))?;
        let path = entry.path();
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
            .ok_or_else(|| format!("{}: not an m<x>_<z> mapsquare", path.display()))?;
        squares.push(coords);
    }
    if squares.is_empty() {
        return Err(format!(
            "canlight maps {}: no m<x>_<z>.jm2 mapsquares",
            maps_dir.display()
        ));
    }
    squares.sort_unstable();
    Ok(squares)
}

fn coverage_plane(collision: &WorldCollision, squares: &[(i32, i32)]) -> Vec<bool> {
    let mut covered = vec![false; collision.width * collision.height];
    for &(square_x, square_z) in squares {
        let ox = square_x * SQUARE - collision.origin.x;
        let oz = square_z * SQUARE - collision.origin.z;
        if ox < 0 || oz < 0 {
            continue;
        }
        let (ox, oz) = (ox as usize, oz as usize);
        for z in 0..SQUARE as usize {
            for x in 0..SQUARE as usize {
                let lx = ox + x;
                let lz = oz + z;
                if lx < collision.width && lz < collision.height {
                    covered[lz * collision.width + lx] = true;
                }
            }
        }
    }
    covered
}

fn stamp_active_loc_coverage(
    collision: &WorldCollision,
    maps_dir: &Path,
    squares: &[(i32, i32)],
    loc_defs: &LocDefs,
    unsafe_locs: &mut [bool],
) -> Result<(), String> {
    for &(square_x, square_z) in squares {
        let path = maps_dir.join(format!("m{square_x}_{square_z}.jm2"));
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("canlight {}: {e}", path.display()))?;
        stamp_square_active_locs(collision, square_x, square_z, &text, loc_defs, unsafe_locs)
            .map_err(|e| format!("canlight {}: {e}", path.display()))?;
    }
    Ok(())
}

fn stamp_square_active_locs(
    collision: &WorldCollision,
    square_x: i32,
    square_z: i32,
    text: &str,
    loc_defs: &LocDefs,
    unsafe_locs: &mut [bool],
) -> Result<(), PackError> {
    let mut in_map = false;
    let mut in_loc = false;
    let mut map_rows = Vec::new();
    let mut locs = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = section(line) {
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
    let link_below: HashSet<(usize, usize)> = map_rows
        .iter()
        .filter(|&&(level, _, _, f)| level == 1 && f & 2 != 0)
        .map(|&(_, x, z, _)| (x, z))
        .collect();
    let plane = collision.width * collision.height;
    for loc in &locs {
        let Some(true_level) = game_plane(loc.level, link_below.contains(&(loc.x, loc.z))) else {
            continue;
        };
        let origin_x = square_x * SQUARE + loc.x as i32;
        let origin_z = square_z * SQUARE + loc.z as i32;
        let def = loc_defs.loc(loc.loc_id);
        // Conservative missing-def: treat as active 1×1 so a hole in the
        // loc table cannot become an all-allowed cell.
        let active = def.is_none_or(|d| d.active);
        if !active {
            continue;
        }
        let (width, length) = def.map_or((1, 1), |d| (d.width.max(1), d.length.max(1)));
        let (span_x, span_z) = if loc.angle == LocAngle::NORTH || loc.angle == LocAngle::SOUTH {
            (length, width)
        } else {
            (width, length)
        };
        for dx in 0..span_x {
            for dz in 0..span_z {
                mark_unsafe(
                    collision,
                    plane,
                    origin_x + dx,
                    origin_z + dz,
                    true_level,
                    unsafe_locs,
                );
            }
        }
    }
    Ok(())
}

fn mark_unsafe(
    collision: &WorldCollision,
    plane: usize,
    x: i32,
    z: i32,
    level: i32,
    unsafe_locs: &mut [bool],
) {
    if !(0..LEVELS).contains(&level) {
        return;
    }
    let lx = x - collision.origin.x;
    let lz = z - collision.origin.z;
    if lx < 0 || lz < 0 {
        return;
    }
    let (lx, lz) = (lx as usize, lz as usize);
    if lx >= collision.width || lz >= collision.height {
        return;
    }
    let idx = level as usize * plane + lz * collision.width + lx;
    if let Some(cell) = unsafe_locs.get_mut(idx) {
        *cell = true;
    }
}

#[cfg(test)]
#[path = "canlight_tests.rs"]
mod tests;
