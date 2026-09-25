//! Nav pack: encode/decode of the whole-world [`WorldCollision`] +
//! [`TransportGraph`] to the `.navpack` binary format, the legacy
//! [`StepGrid`] grid format, plus the jm2 mapsquare bake used by the
//! `nav-pack` binary.
//!
//! Grid format (274N): magic `b"274N"`, version `u8` 1, origin
//! `(x, z, level)` i32le, width/height u32le, one walk byte per tile
//! (row-major z then x, 1 = walkable, same indexing as [`StepGrid`]),
//! door count u32le, then per door `(loc_x, loc_z, loc_level, loc_id,
//! from_x, from_z, from_level, to_x, to_z, to_level)` all i32le. Door loc
//! ids come from the Server `content/scripts/doors/configs/*.loc` blocks
//! (see [`parse_door_config`]). Blocking loc footprints come from
//! `[loc_N]` `blockwalk` (default yes).
//!
//! Pack format (274V): magic `b"274V"`, version `u8` 10, collision origin
//! `(x, z, level)` i32le, width/height u32le, the [`WorldCollision`]
//! packed walk surface — first the `u8` face byte per tile per level,
//! four planes, level-major, each `width × height` (row-major z then x),
//! then the `SQ_BLOCKED` bit-plane as `u64le` words, 64 cells per word,
//! same indexing — then the transport edge
//! count u32le and per edge `(kind u8, at x/z/level, to x/z/level,
//! loc_id, option, ticks, dir u8, open_loc_id)` i32le plus the five
//! requirement vectors (count u32le, then `(id, value)` i32le pairs;
//! quest names as length-prefixed UTF-8; `worn_req` as plain i32le ids;
//! then `members_req` as a `u8` `0`/`1`). `dir` encodes [`DoorDir`] as `0=None,
//! 1=N, 2=E, 3=S, 4=W`; `open_loc_id` is `-1` for `None`. The any-tile
//! teleport layer (`TransportGraph::teleports`) round-trips inside
//! the same edges array as kind-4 edges; [`decode`] splits them back out
//! and never indexes them into `at`. The raw flags are not on the v8 wire
//! — the flags sidecar is separate: magic `b"274F"`, version 1, the same
//! origin/width/height header as the pack, then the level-major u32le flags
//! ([`encode_flags_sidecar`]/[`decode_flags_sidecar`]). The paint-reach
//! bitset is a second sidecar, not on the v8 pack wire: magic `b"274R"`,
//! version 1, the same origin/width/height header, an explicit word count,
//! a 32-byte pack-identity binding, then `u64le` words
//! ([`encode_reach_sidecar`]/[`decode_reach_sidecar`]). The static canlight
//! bitset is a third sidecar, same header dialect: magic `b"274L"`, version 1
//! ([`encode_canlight_sidecar`]/[`decode_canlight_sidecar`]); its 32-byte
//! binding is pack identity concatenated with canlight policy identity, not
//! pack SHA alone. After the edges,
//! v8 appends the content-derived bank stand table: count u32le, then per
//! stand a length-prefixed name, the `x/z/level` tile i32le, and the
//! access (`u8` tag: 0 = [`BankAccess::Booth`] `op` i32le, 1 =
//! [`BankAccess::Npc`] length-prefixed npc name + `op` i32le + an optional
//! dialog choice as a presence `u8` then a length-prefixed string), see
//! [`derive_banks`]. [`decode`] accepts version 10 only — a v9 stream (or
//! any earlier one, the v6 packed u16 words included) is
//! [`PackError::BadVersion`];
//! there is no flags→walk compat load. The 274N grid decoder stays for
//! old `.navpack` files; `nav-pack` now writes v10.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::io::{self, Cursor, Read};
use std::path::Path;

use api::snapshot::WorldTile;

use crate::collision::WorldCollision;
use crate::grid::{DoorEdge, StepGrid};
use crate::tile::Tile;
use crate::transport::{DoorDir, TransportEdge, TransportGraph, TransportKind};

mod banks;
mod config_parse;
mod mapsquare;
mod sidecars;

pub use banks::{derive_banks, BankAccess, BankStand};
use banks::{read_bank_stands, write_bank_stands};
pub use config_parse::{
    parse_door_config, parse_door_config_ids, parse_door_open_ids, parse_passable_locs,
};
#[expect(unused_imports)]
pub(crate) use mapsquare::LocOnSquare;
pub use mapsquare::{merge_squares, parse_mapsquare_jm2, walkable_dots, Mapsquare};
pub(crate) use mapsquare::{parse_loc_fields, parse_map_line, section};
#[cfg(test)]
use mapsquare::{parse_mapsquare_text, SQUARE};
pub use sidecars::{
    decode_canlight_sidecar, decode_flags_sidecar, decode_flags_sidecar_arc, decode_reach_sidecar,
    encode_canlight_sidecar, encode_flags_sidecar, encode_reach_sidecar, read_flags_sidecar,
    sha256_from_hex, sha256_hex, CanlightSidecar, FlagsSidecarLoad, ReachSidecar, FLAGS_HEADER_LEN,
};
#[cfg(test)]
use sidecars::{MAGIC_FLAGS, VERSION_FLAGS};

/// Grid (274N) format version: boolean walk bytes + doors.
const VERSION_GRID: u8 = 1;
/// Grid (274N) file magic.
const MAGIC_GRID: &[u8; 4] = b"274N";
/// Current pack format version (collision + transport graph). v3 adds the
/// per-edge `dir`/`open_loc_id` fields, v4 stores the four collision
/// planes, v5 adds the per-edge worn-item id list `worn_req`, and v6 —
/// the packed-walk wire — replaces the four u32 flag planes with the
/// compact packed u16 walk words (no resident u32 flags; the flags
/// sidecar is separate). v7 splits that u16 walk
/// word into the `u8` face byte per cell plus the packed `SQ_BLOCKED`
/// bit-plane (9 bits per cell instead of 16). v8 appends the
/// content-derived bank stand table ([`BankStand`], baked by
/// [`derive_banks`]) after the transport edges; the v4 wire also carries
/// the spirit-tree (7) and reserved NPC (8) transport kinds on the same
/// kind byte — no version bump. v9 appends a per-edge `members_req` `u8`
/// (`0`/`1`) after `worn_req`. v10 — the current wire — appends a per-edge
/// wilderness teleport cap (`i32le`, `-1` = none) after `members_req` and
/// the wilderness-level formula after the bank table. [`decode`] accepts
/// version 10 only; 9, 8, 7, 6, 5, and older streams are rejected rather
/// than compat-loaded.
/// Rebake with `nav-pack` over `$ENGINE_DIR/../content/maps` whenever the
/// Server content changes (new loc/NPC placements, pack bumps).
const VERSION: u8 = 10;
/// Current pack file magic.
const MAGIC: &[u8; 4] = b"274V";
/// Pack format identity as it appears in bundled navigation identities: the
/// file magic followed by the format version (`274V10` for the current wire).
/// A format improvement changes this identity and therefore invalidates
/// staged build artifacts.
pub const FORMAT_ID: &str = "274V10";
/// Bytes per door entry.
const DOOR_BYTES: usize = 40;
/// Largest grid side a pack may decode (16384×16384 tiles ≈ 256 MB of walk
/// bytes; the whole-world bbox is 1792×9088, comfortably under).
const MAX_GRID: usize = 16384;

/// Errors loading, writing, or baking a nav pack.
#[derive(Debug)]
pub enum PackError {
    /// Filesystem read/write failure.
    Io(io::Error),
    /// File does not start with the `b"274N"` or `b"274V"` magic.
    BadMagic,
    /// Pack version is not the expected one for its magic.
    BadVersion(u8),
    /// File ended before the declared contents.
    Truncated,
    /// Declared grid or door count is inconsistent.
    BadLength(String),
}

impl fmt::Display for PackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PackError::Io(e) => write!(f, "io error: {e}"),
            PackError::BadMagic => write!(f, "bad pack magic (expected b\"274N\" or b\"274V\")"),
            PackError::BadVersion(v) => write!(f, "unsupported pack version {v}"),
            PackError::Truncated => write!(f, "pack file truncated"),
            PackError::BadLength(m) => write!(f, "inconsistent pack: {m}"),
        }
    }
}

impl std::error::Error for PackError {}

/// Serialize `g` to the 274N grid byte format.
pub fn encode_grid(g: &StepGrid) -> Vec<u8> {
    let mut out =
        Vec::with_capacity(4 + 1 + 12 + 8 + g.walk.len() + 4 + g.doors.len() * DOOR_BYTES);
    out.extend_from_slice(MAGIC_GRID);
    out.push(VERSION_GRID);
    for v in [g.origin.x, g.origin.z, g.origin.level] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&(g.width as u32).to_le_bytes());
    out.extend_from_slice(&(g.height as u32).to_le_bytes());
    out.extend_from_slice(&g.walk);
    out.extend_from_slice(&(g.doors.len() as u32).to_le_bytes());
    for d in &g.doors {
        for v in [
            d.loc.x,
            d.loc.z,
            d.loc.level,
            d.loc_id,
            d.from.x,
            d.from.z,
            d.from.level,
            d.to.x,
            d.to.z,
            d.to.level,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out
}

/// Deserialize a 274N grid, validating magic, version, and lengths.
pub fn decode_grid(bytes: &[u8]) -> Result<StepGrid, PackError> {
    let mut r = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    r.read_exact(&mut magic).map_err(|_| PackError::Truncated)?;
    if &magic != MAGIC_GRID {
        return Err(PackError::BadMagic);
    }
    let mut version = [0u8; 1];
    r.read_exact(&mut version)
        .map_err(|_| PackError::Truncated)?;
    if version[0] != VERSION_GRID {
        return Err(PackError::BadVersion(version[0]));
    }
    let origin = Tile {
        x: read_i32(&mut r)?,
        z: read_i32(&mut r)?,
        level: read_i32(&mut r)?,
    };
    let width = read_u32(&mut r)? as usize;
    let height = read_u32(&mut r)? as usize;
    if width == 0 || height == 0 || width > MAX_GRID || height > MAX_GRID {
        return Err(PackError::BadLength(format!(
            "grid {width}x{height} exceeds the {MAX_GRID} tile cap"
        )));
    }
    let cells = width
        .checked_mul(height)
        .ok_or_else(|| PackError::BadLength("grid size overflows".into()))?;
    let mut walk = vec![0u8; cells];
    r.read_exact(&mut walk).map_err(|_| PackError::Truncated)?;
    let n_doors = read_u32(&mut r)? as usize;
    // Cap the preallocation at what the remaining bytes can hold; the reads
    // themselves still fail with Truncated past the real end.
    let remaining = bytes.len().saturating_sub(r.position() as usize);
    let mut doors = Vec::with_capacity(n_doors.min(remaining / DOOR_BYTES));
    for _ in 0..n_doors {
        doors.push(DoorEdge {
            loc: Tile {
                x: read_i32(&mut r)?,
                z: read_i32(&mut r)?,
                level: read_i32(&mut r)?,
            },
            loc_id: read_i32(&mut r)?,
            from: Tile {
                x: read_i32(&mut r)?,
                z: read_i32(&mut r)?,
                level: read_i32(&mut r)?,
            },
            to: Tile {
                x: read_i32(&mut r)?,
                z: read_i32(&mut r)?,
                level: read_i32(&mut r)?,
            },
        });
    }
    Ok(StepGrid::from_parts(origin, width, height, walk, doors))
}

/// Read and decode the 274N grid at `path`.
pub fn load_grid(path: &Path) -> Result<StepGrid, PackError> {
    let bytes = std::fs::read(path).map_err(PackError::Io)?;
    decode_grid(&bytes)
}

/// Serialize the whole-world collision + transport graph + bank stand
/// table to the v8 pack byte format. The graph's `at` index is not
/// stored; [`decode`]
/// rebuilds it from the edges. Teleports (kind-4 edges) are written after
/// the ordinary edges in the same array. The v8 wire (version byte 8)
/// carries the [`WorldCollision`] as the `u8` face bytes per tile per
/// level (four level-major planes) plus the packed `SQ_BLOCKED`
/// bit-plane, the per-edge `worn_req` id list, and the trailing bank
/// stand table (see [`BankStand`]); the raw flags are
/// not resident and not on the wire (see the flags sidecar).
pub fn encode(collision: &WorldCollision, graph: &TransportGraph, banks: &[BankStand]) -> Vec<u8> {
    let edge_count = graph.edges.len() + graph.teleports.len();
    let mut out = Vec::with_capacity(
        4 + 1
            + 12
            + 8
            + collision.walk.len()
            + collision.blocked.len() * 8
            + 4
            + edge_count * 96
            + 4
            + banks.len() * 48,
    );
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    for v in [
        collision.origin.x,
        collision.origin.z,
        collision.origin.level,
    ] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&(collision.width as u32).to_le_bytes());
    out.extend_from_slice(&(collision.height as u32).to_le_bytes());
    out.extend_from_slice(&collision.walk);
    for w in &collision.blocked {
        out.extend_from_slice(&w.to_le_bytes());
    }
    out.extend_from_slice(&(edge_count as u32).to_le_bytes());
    for e in graph.edges.iter().chain(&graph.teleports) {
        out.push(kind_to_u8(e.kind));
        for v in [
            e.at.x, e.at.z, e.at.level, e.to.x, e.to.z, e.to.level, e.loc_id, e.option, e.ticks,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.push(dir_to_u8(e.dir));
        out.extend_from_slice(&e.open_loc_id.unwrap_or(-1).to_le_bytes());
        write_req_pairs(&mut out, &e.skill_req);
        write_req_pairs(&mut out, &e.item_req);
        write_req_strings(&mut out, &e.quest_req);
        write_req_pairs(&mut out, &e.varp_req);
        write_req_ids(&mut out, &e.worn_req);
        out.push(if e.members_req { 1 } else { 0 });
        let cap = e.wildy_cap.unwrap_or(-1);
        out.extend_from_slice(&cap.to_le_bytes());
    }
    write_bank_stands(&mut out, banks);
    write_wilderness_rules(&mut out, &graph.wilderness);
    out
}

/// Deserialize the whole-world pack, validating magic, version, and
/// lengths. The `at` index is rebuilt from the decoded edges; kind-4
/// (teleport) edges split back into [`TransportGraph::teleports`] and are
/// excluded from it. Version 10 is the only accepted wire: the collision
/// decodes as the `u8` face bytes plus the packed `SQ_BLOCKED`
/// bit-plane with no resident flags (`flags` is
/// `None` until the sidecar is loaded), and the trailing bank stand table
/// (see [`BankStand`]) decodes after the edges; any other version — 9, 8, 7, 6,
/// 5, or older — is rejected rather than mis-read or compat-loaded.
pub fn decode(bytes: &[u8]) -> Result<(WorldCollision, TransportGraph, Vec<BankStand>), PackError> {
    let mut r = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    r.read_exact(&mut magic).map_err(|_| PackError::Truncated)?;
    if &magic != MAGIC {
        return Err(PackError::BadMagic);
    }
    let mut version = [0u8; 1];
    r.read_exact(&mut version)
        .map_err(|_| PackError::Truncated)?;
    if version[0] != VERSION {
        return Err(PackError::BadVersion(version[0]));
    }
    let origin = WorldTile {
        x: read_i32(&mut r)?,
        z: read_i32(&mut r)?,
        level: read_i32(&mut r)?,
    };
    let width = read_u32(&mut r)? as usize;
    let height = read_u32(&mut r)? as usize;
    if width == 0 || height == 0 || width > MAX_GRID || height > MAX_GRID {
        return Err(PackError::BadLength(format!(
            "grid {width}x{height} exceeds the {MAX_GRID} tile cap"
        )));
    }
    let plane = width
        .checked_mul(height)
        .ok_or_else(|| PackError::BadLength("grid size overflows".into()))?;
    let cells = plane
        .checked_mul(4)
        .ok_or_else(|| PackError::BadLength("grid size overflows".into()))?;
    let mut walk = vec![0u8; cells];
    r.read_exact(&mut walk).map_err(|_| PackError::Truncated)?;
    let words = cells.div_ceil(64);
    let remaining = bytes.len().saturating_sub(r.position() as usize);
    let mut blocked = Vec::with_capacity(words.min(remaining / 8));
    for _ in 0..words {
        blocked.push(read_u64(&mut r)?);
    }
    let n_edges = read_u32(&mut r)? as usize;
    // Cap the preallocation at what the remaining bytes can hold; the reads
    // themselves still fail with Truncated past the real end.
    let remaining = bytes.len().saturating_sub(r.position() as usize);
    let mut graph = TransportGraph {
        edges: Vec::with_capacity(n_edges.min(remaining / 41)),
        ..Default::default()
    };
    for _ in 0..n_edges {
        let mut kind = [0u8; 1];
        r.read_exact(&mut kind).map_err(|_| PackError::Truncated)?;
        let edge = TransportEdge {
            kind: kind_from_u8(kind[0])?,
            at: WorldTile {
                x: read_i32(&mut r)?,
                z: read_i32(&mut r)?,
                level: read_i32(&mut r)?,
            },
            to: WorldTile {
                x: read_i32(&mut r)?,
                z: read_i32(&mut r)?,
                level: read_i32(&mut r)?,
            },
            loc_id: read_i32(&mut r)?,
            option: read_i32(&mut r)?,
            ticks: read_i32(&mut r)?,
            dir: dir_from_u8(read_u8(&mut r)?)?,
            open_loc_id: {
                let id = read_i32(&mut r)?;
                if id == -1 {
                    None
                } else {
                    Some(id)
                }
            },
            skill_req: read_req_pairs(&mut r)?,
            item_req: read_req_pairs(&mut r)?,
            quest_req: read_req_strings(&mut r)?,
            varp_req: read_req_pairs(&mut r)?,
            worn_req: read_req_ids(&mut r)?,
            members_req: match read_u8(&mut r)? {
                0 => false,
                1 => true,
                other => {
                    return Err(PackError::BadLength(format!(
                        "members_req flag {other} is not 0 or 1"
                    )));
                }
            },
            wildy_cap: {
                let cap = read_i32(&mut r)?;
                if cap < 0 {
                    if cap != -1 {
                        return Err(PackError::BadLength(format!(
                            "wildy_cap {cap} is not -1 or a non-negative level"
                        )));
                    }
                    None
                } else {
                    Some(cap)
                }
            },
        };
        if edge.kind == TransportKind::Teleport {
            graph.teleports.push(edge);
        } else {
            graph.edges.push(edge);
        }
    }
    for (i, e) in graph.edges.iter().enumerate() {
        graph.at.entry(e.at).or_default().push(i);
    }
    let banks = read_bank_stands(&mut r)?;
    graph.wilderness = read_wilderness_rules(&mut r)?;
    Ok((
        WorldCollision {
            origin,
            width,
            height,
            // The packed walk surface is the resident form; the raw flags
            // live only in the sidecar (loaded on demand for debug paints).
            walk,
            blocked,
            flags: None,
        },
        graph,
        banks,
    ))
}

/// `TransportKind` as a wire byte.
fn kind_to_u8(k: TransportKind) -> u8 {
    match k {
        TransportKind::Door => 0,
        TransportKind::Ladder => 1,
        TransportKind::Stairs => 2,
        TransportKind::Boat => 3,
        TransportKind::Teleport => 4,
        TransportKind::AgilityShortcut => 5,
        TransportKind::Glider => 6,
        TransportKind::SpiritTree => 7,
        TransportKind::Npc => 8,
        // The essence-mine return hop is synthesized per-slot from the
        // live EssenceSession — never packed, so encode never sees it
        // (decode rejects the byte too, keeping it off the wire).
        TransportKind::EssenceExit => unreachable!("the essence return is never packed"),
    }
}

/// Wire byte → [`TransportKind`], rejecting unknown values.
fn kind_from_u8(b: u8) -> Result<TransportKind, PackError> {
    match b {
        0 => Ok(TransportKind::Door),
        1 => Ok(TransportKind::Ladder),
        2 => Ok(TransportKind::Stairs),
        3 => Ok(TransportKind::Boat),
        4 => Ok(TransportKind::Teleport),
        5 => Ok(TransportKind::AgilityShortcut),
        6 => Ok(TransportKind::Glider),
        7 => Ok(TransportKind::SpiritTree),
        8 => Ok(TransportKind::Npc),
        _ => Err(PackError::BadLength(format!("unknown transport kind {b}"))),
    }
}

/// `Option<DoorDir>` as a wire byte: `0=None, 1=N, 2=E, 3=S, 4=W`.
fn dir_to_u8(d: Option<DoorDir>) -> u8 {
    match d {
        None => 0,
        Some(DoorDir::N) => 1,
        Some(DoorDir::E) => 2,
        Some(DoorDir::S) => 3,
        Some(DoorDir::W) => 4,
    }
}

/// Wire byte → `Option<DoorDir>` (`0` = None), rejecting unknown values.
fn dir_from_u8(b: u8) -> Result<Option<DoorDir>, PackError> {
    match b {
        0 => Ok(None),
        1 => Ok(Some(DoorDir::N)),
        2 => Ok(Some(DoorDir::E)),
        3 => Ok(Some(DoorDir::S)),
        4 => Ok(Some(DoorDir::W)),
        _ => Err(PackError::BadLength(format!("unknown door dir {b}"))),
    }
}

/// A requirement vector as `(id, value)` i32le pairs, count-prefixed.
fn write_req_pairs(out: &mut Vec<u8>, reqs: &[(i32, i32)]) {
    out.extend_from_slice(&(reqs.len() as u32).to_le_bytes());
    for (a, b) in reqs {
        out.extend_from_slice(&a.to_le_bytes());
        out.extend_from_slice(&b.to_le_bytes());
    }
}

/// A quest-name vector as length-prefixed UTF-8 strings, count-prefixed.
fn write_req_strings(out: &mut Vec<u8>, reqs: &[String]) {
    out.extend_from_slice(&(reqs.len() as u32).to_le_bytes());
    for s in reqs {
        out.extend_from_slice(&(s.len() as u32).to_le_bytes());
        out.extend_from_slice(s.as_bytes());
    }
}

/// Read a count-prefixed `(id, value)` pair vector.
fn read_req_pairs(r: &mut Cursor<&[u8]>) -> Result<Vec<(i32, i32)>, PackError> {
    let n = read_u32(r)? as usize;
    let remaining = r.get_ref().len().saturating_sub(r.position() as usize);
    let mut out = Vec::with_capacity(n.min(remaining / 8));
    for _ in 0..n {
        out.push((read_i32(r)?, read_i32(r)?));
    }
    Ok(out)
}

/// Read a count-prefixed length-prefixed UTF-8 string vector.
fn read_req_strings(r: &mut Cursor<&[u8]>) -> Result<Vec<String>, PackError> {
    let n = read_u32(r)? as usize;
    let remaining = r.get_ref().len().saturating_sub(r.position() as usize);
    let mut out = Vec::with_capacity(n.min(remaining / 4));
    for _ in 0..n {
        let len = read_u32(r)? as usize;
        let mut buf = vec![0u8; len];
        r.read_exact(&mut buf).map_err(|_| PackError::Truncated)?;
        let s = String::from_utf8(buf)
            .map_err(|_| PackError::BadLength("quest req is not UTF-8".into()))?;
        out.push(s);
    }
    Ok(out)
}

/// A worn-item requirement vector as i32le ids, count-prefixed.
fn write_req_ids(out: &mut Vec<u8>, reqs: &[i32]) {
    out.extend_from_slice(&(reqs.len() as u32).to_le_bytes());
    for id in reqs {
        out.extend_from_slice(&id.to_le_bytes());
    }
}

/// Read a count-prefixed i32le id vector (the `worn_req` list).
fn read_req_ids(r: &mut Cursor<&[u8]>) -> Result<Vec<i32>, PackError> {
    let n = read_u32(r)? as usize;
    let remaining = r.get_ref().len().saturating_sub(r.position() as usize);
    let mut out = Vec::with_capacity(n.min(remaining / 4));
    for _ in 0..n {
        out.push(read_i32(r)?);
    }
    Ok(out)
}

fn write_wilderness_rules(out: &mut Vec<u8>, rules: &crate::transport::WildernessRules) {
    out.extend_from_slice(&(rules.zones.len() as u32).to_le_bytes());
    for z in &rules.zones {
        for v in [z.x1, z.z1, z.x2, z.z2, z.level1, z.level2, z.origin_z] {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out.extend_from_slice(&rules.divisor.to_le_bytes());
    out.extend_from_slice(&rules.offset.to_le_bytes());
}

fn read_wilderness_rules(
    r: &mut Cursor<&[u8]>,
) -> Result<crate::transport::WildernessRules, PackError> {
    let n = read_u32(r)? as usize;
    let remaining = r.get_ref().len().saturating_sub(r.position() as usize);
    if n > remaining / 28 {
        return Err(PackError::BadLength(format!(
            "wilderness zone count {n} exceeds remaining pack bytes"
        )));
    }
    let mut zones = Vec::with_capacity(n);
    for _ in 0..n {
        zones.push(crate::transport::WildernessZone {
            x1: read_i32(r)?,
            z1: read_i32(r)?,
            x2: read_i32(r)?,
            z2: read_i32(r)?,
            level1: read_i32(r)?,
            level2: read_i32(r)?,
            origin_z: read_i32(r)?,
        });
    }
    Ok(crate::transport::WildernessRules {
        zones,
        divisor: read_i32(r)?,
        offset: read_i32(r)?,
    })
}

fn read_i32(r: &mut Cursor<&[u8]>) -> Result<i32, PackError> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b).map_err(|_| PackError::Truncated)?;
    Ok(i32::from_le_bytes(b))
}

fn read_u8(r: &mut Cursor<&[u8]>) -> Result<u8, PackError> {
    let mut b = [0u8; 1];
    r.read_exact(&mut b).map_err(|_| PackError::Truncated)?;
    Ok(b[0])
}

fn read_u32(r: &mut Cursor<&[u8]>) -> Result<u32, PackError> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b).map_err(|_| PackError::Truncated)?;
    Ok(u32::from_le_bytes(b))
}

fn read_u64(r: &mut Cursor<&[u8]>) -> Result<u64, PackError> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b).map_err(|_| PackError::Truncated)?;
    Ok(u64::from_le_bytes(b))
}

#[cfg(test)]
#[path = "pack_tests.rs"]
mod tests;
