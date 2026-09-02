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
//! Pack format (274V): magic `b"274V"`, version `u8` 8, collision origin
//! `(x, z, level)` i32le, width/height u32le, the [`WorldCollision`]
//! packed walk surface — first the `u8` face byte per tile per level,
//! four planes, level-major, each `width × height` (row-major z then x),
//! then the `SQ_BLOCKED` bit-plane as `u64le` words, 64 cells per word,
//! same indexing — then the transport edge
//! count u32le and per edge `(kind u8, at x/z/level, to x/z/level,
//! loc_id, option, ticks, dir u8, open_loc_id)` i32le plus the five
//! requirement vectors (count u32le, then `(id, value)` i32le pairs;
//! quest names as length-prefixed UTF-8; `worn_req` as plain i32le ids).
//! `dir` encodes [`DoorDir`] as `0=None,
//! 1=N, 2=E, 3=S, 4=W`; `open_loc_id` is `-1` for `None`. The any-tile
//! teleport layer (`TransportGraph::teleports`) round-trips inside
//! the same edges array as kind-4 edges; [`decode`] splits them back out
//! and never indexes them into `at`. The raw flags are not on the v8 wire
//! — the flags sidecar is separate: magic `b"274F"`, version 1, the same
//! origin/width/height header as the pack, then the level-major u32le flags
//! ([`encode_flags_sidecar`]/[`decode_flags_sidecar`]). After the edges,
//! v8 appends the content-derived bank stand table: count u32le, then per
//! stand a length-prefixed name, the `x/z/level` tile i32le, and the
//! access (`u8` tag: 0 = [`BankAccess::Booth`] `op` i32le, 1 =
//! [`BankAccess::Npc`] length-prefixed npc name + `op` i32le + an optional
//! dialog choice as a presence `u8` then a length-prefixed string), see
//! [`derive_banks`]. [`decode`] accepts version 8 only — a v7 stream (or
//! any earlier one, the v6 packed u16 words included) is
//! [`PackError::BadVersion`];
//! there is no flags→walk compat load. The 274N grid decoder stays for
//! old `.navpack` files; `nav-pack` now writes v8.

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
/// bit-plane (9 bits per cell instead of 16). v8 — the current wire —
/// appends the content-derived bank stand table ([`BankStand`], baked by
/// [`derive_banks`]) after the transport edges; the v4 wire also carries
/// the spirit-tree (7) and reserved NPC (8) transport kinds on the same
/// kind byte — no version bump. [`decode`] accepts version 8 only; 7, 6,
/// 5, and older streams are rejected rather than compat-loaded.
/// Rebake with `nav-pack` over `$ENGINE_DIR/../content/maps` whenever the
/// Server content changes (new loc/NPC placements, pack bumps).
const VERSION: u8 = 8;
/// Current pack file magic.
const MAGIC: &[u8; 4] = b"274V";
/// Flags sidecar format version.
const VERSION_FLAGS: u8 = 1;
/// Flags sidecar magic.
const MAGIC_FLAGS: &[u8; 4] = b"274F";
/// Mapsquare edge length in tiles.
const SQUARE: usize = 64;
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

/// One bank stand on the v8 wire: a named interact target — either a
/// booth loc or a teller NPC — that opens a bank. The router's banking
/// session walks to `tile` and uses the `access` op on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BankStand {
    /// The stand's display name ("Bank booth", the teller's NPC name).
    pub name: String,
    /// The interact tile (the booth loc tile or the NPC's tile).
    pub tile: WorldTile,
    /// How the stand is used.
    pub access: BankAccess,
}

/// How a [`BankStand`] is activated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BankAccess {
    /// A `bankbooth` loc: use `op` on it to open the bank (2 = the
    /// `Use-quickly` op of `scripts/interface_bank/configs/bank_booth.loc`).
    Booth { op: i32 },
    /// A teller NPC: use `op` on the named NPC to open the bank
    /// (`choose` is the dialog option text when the op itself only starts
    /// the dialogue, not the bank).
    Npc {
        name: String,
        op: i32,
        choose: Option<String>,
    },
}

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
    }
    write_bank_stands(&mut out, banks);
    out
}

/// Deserialize the whole-world pack, validating magic, version, and
/// lengths. The `at` index is rebuilt from the decoded edges; kind-4
/// (teleport) edges split back into [`TransportGraph::teleports`] and are
/// excluded from it. Version 8 is the only accepted wire: the collision
/// decodes as the `u8` face bytes plus the packed `SQ_BLOCKED`
/// bit-plane with no resident flags (`flags` is
/// `None` until the sidecar is loaded), and the trailing bank stand table
/// (see [`BankStand`]) decodes after the edges; any other version — 7, 6,
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

/// Serialize the raw baked flags to the sidecar byte format: magic
/// `b"274F"`, version 1, the same origin/width/height header as the pack,
/// then
/// the level-major u32le flags. The flag count is implicit — the trailing
/// bytes are the flags, so a `width × height` test grid round-trips
/// without plane arithmetic ([`decode_flags_sidecar`] reads to the end).
pub fn encode_flags_sidecar(
    origin: WorldTile,
    width: usize,
    height: usize,
    flags: &[u32],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + 1 + 12 + 8 + flags.len() * 4);
    out.extend_from_slice(MAGIC_FLAGS);
    out.push(VERSION_FLAGS);
    for v in [origin.x, origin.z, origin.level] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&(width as u32).to_le_bytes());
    out.extend_from_slice(&(height as u32).to_le_bytes());
    for f in flags {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

/// Deserialize a flags sidecar, validating magic, version, and the grid
/// header, then reading the trailing u32le flags to the end of the
/// buffer (a partial trailing u32 is [`PackError::Truncated`]).
pub fn decode_flags_sidecar(
    bytes: &[u8],
) -> Result<(WorldTile, usize, usize, Vec<u32>), PackError> {
    let mut r = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    r.read_exact(&mut magic).map_err(|_| PackError::Truncated)?;
    if &magic != MAGIC_FLAGS {
        return Err(PackError::BadMagic);
    }
    let mut version = [0u8; 1];
    r.read_exact(&mut version)
        .map_err(|_| PackError::Truncated)?;
    if version[0] != VERSION_FLAGS {
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
    let remaining = bytes.len().saturating_sub(r.position() as usize);
    if !remaining.is_multiple_of(4) {
        return Err(PackError::Truncated);
    }
    let mut flags = Vec::with_capacity(remaining / 4);
    for _ in 0..remaining / 4 {
        flags.push(read_u32(&mut r)?);
    }
    Ok((origin, width, height, flags))
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

/// Fewest bytes one [`BankStand`] can occupy on the v8 wire (name len
/// prefix + empty name + tile + access tag + op) — a preallocation cap.
const MIN_BANK_BYTES: usize = 4 + 12 + 1 + 4;

/// The `[bankbooth]` block of `scripts/interface_bank/configs/bank_booth.loc`.
const BANK_BOOTH_CONFIG: &str = "scripts/interface_bank/configs/bank_booth.loc";

/// Write the bank stand table: count u32le, then per stand a
/// length-prefixed name, the `x/z/level` tile i32le, and the access (u8
/// tag 0 = Booth `op` i32le, 1 = Npc length-prefixed name + `op` i32le +
/// an optional dialog choice: presence u8 then a length-prefixed string).
fn write_bank_stands(out: &mut Vec<u8>, banks: &[BankStand]) {
    out.extend_from_slice(&(banks.len() as u32).to_le_bytes());
    for b in banks {
        write_name(out, &b.name);
        for v in [b.tile.x, b.tile.z, b.tile.level] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        match &b.access {
            BankAccess::Booth { op } => {
                out.push(0);
                out.extend_from_slice(&op.to_le_bytes());
            }
            BankAccess::Npc { name, op, choose } => {
                out.push(1);
                write_name(out, name);
                out.extend_from_slice(&op.to_le_bytes());
                match choose {
                    Some(c) => {
                        out.push(1);
                        write_name(out, c);
                    }
                    None => out.push(0),
                }
            }
        }
    }
}

/// Read the bank stand table written by [`write_bank_stands`].
fn read_bank_stands(r: &mut Cursor<&[u8]>) -> Result<Vec<BankStand>, PackError> {
    let n = read_u32(r)? as usize;
    let remaining = r.get_ref().len().saturating_sub(r.position() as usize);
    let mut out = Vec::with_capacity(n.min(remaining / MIN_BANK_BYTES));
    for _ in 0..n {
        let name = read_name(r)?;
        let tile = WorldTile {
            x: read_i32(r)?,
            z: read_i32(r)?,
            level: read_i32(r)?,
        };
        let access = match read_u8(r)? {
            0 => BankAccess::Booth { op: read_i32(r)? },
            1 => {
                let npc = read_name(r)?;
                let op = read_i32(r)?;
                let choose = if read_u8(r)? != 0 {
                    Some(read_name(r)?)
                } else {
                    None
                };
                BankAccess::Npc {
                    name: npc,
                    op,
                    choose,
                }
            }
            tag => {
                return Err(PackError::BadLength(format!(
                    "unknown bank access tag {tag}"
                )))
            }
        };
        out.push(BankStand { name, tile, access });
    }
    Ok(out)
}

/// A length-prefixed UTF-8 string (the bank stand name fields).
fn write_name(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u32).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

/// Read a length-prefixed UTF-8 string (see [`write_name`]).
fn read_name(r: &mut Cursor<&[u8]>) -> Result<String, PackError> {
    let len = read_u32(r)? as usize;
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).map_err(|_| PackError::Truncated)?;
    String::from_utf8(buf).map_err(|_| PackError::BadLength("bank stand name is not UTF-8".into()))
}

/// Bake the bank stand table from the Server content tree (the maps
/// dir's parent): the same jm2 LOC pass the collision bake uses
/// ([`crate::transport`]'s placement reader). Every `bankbooth` loc
/// placement becomes a [`BankStand::Booth`] stand — named from the
/// `[bankbooth]` block of `scripts/interface_bank/configs/bank_booth.loc`
/// and accessed with the Use-quickly op (2, `[oploc2,bankbooth]`). The
/// closed-booth (`bankboothclosed`) and tutorial (`newbiebankbooth`) loc
/// ids are never looked up, so they cannot enter the table; NPC teller
/// stands (`category=bank_teller`) join when a bake parses the jm2 NPC
/// placements — booth-only for now. Stands sort by tile for a
/// deterministic wire.
pub fn derive_banks(content_root: &Path) -> Vec<BankStand> {
    let ids = crate::transport::loc_ids_by_name(content_root);
    let Some(&booth_id) = ids.get("bankbooth") else {
        return Vec::new();
    };
    let name = bank_booth_name(content_root);
    let positions = crate::transport::loc_positions(content_root);
    let mut banks: Vec<BankStand> = positions
        .get(&booth_id)
        .map(|placements| {
            placements
                .iter()
                .map(|p| BankStand {
                    name: name.clone(),
                    tile: WorldTile {
                        x: p.x,
                        z: p.z,
                        level: p.level,
                    },
                    access: BankAccess::Booth { op: 2 },
                })
                .collect()
        })
        .unwrap_or_default();
    banks.sort_by_key(|b| (b.tile.level, b.tile.x, b.tile.z));
    banks
}

/// The `name=` of the `[bankbooth]` block (the booth loc config's display
/// name), `"Bank booth"` when the config is missing.
fn bank_booth_name(content_root: &Path) -> String {
    let Ok(text) = fs::read_to_string(content_root.join(BANK_BOOTH_CONFIG)) else {
        return "Bank booth".to_string();
    };
    let mut in_block = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_block = line == "[bankbooth]";
            continue;
        }
        if in_block {
            if let Some(v) = line.strip_prefix("name=") {
                let v = v.trim();
                if !v.is_empty() {
                    return v.to_string();
                }
            }
        }
    }
    "Bank booth".to_string()
}

/// All walkable tiles of `grid` on `level`, in row-major (z then x) order.
/// Tiles off `grid`'s own level yield an empty iterator.
pub fn walkable_dots(grid: &StepGrid, level: i32) -> impl Iterator<Item = Tile> + '_ {
    let (ox, oz) = (grid.origin.x, grid.origin.z);
    (0..grid.height)
        .flat_map(move |z| {
            (0..grid.width).map(move |x| Tile {
                x: ox + x as i32,
                z: oz + z as i32,
                level,
            })
        })
        .filter(move |t| grid.walkable(*t))
}

/// A baked mapsquare: absolute mapsquare x/z, 64×64 level-0 walk flags
/// (row-major z then x, 1 = walkable), and door edges.
pub struct Mapsquare {
    pub x: i32,
    pub z: i32,
    pub walk: Vec<u8>,
    pub doors: Vec<DoorEdge>,
}

/// Door loc ids from `content/scripts/doors/configs/*.loc` text: every
/// `[loc_N]` block that can open, i.e. has `op1=Open` or
/// `category=door_closed`/`category=gate_main_closed`/
/// `category=gate_outer_closed` (the fence-gate closed categories, same
/// openability as `door_closed`). Non-numeric blocks (e.g. `[membergatel]`)
/// are ignored, as are the `op1=Close`/`*_open` counterpart states.
pub fn parse_door_config(text: &str) -> HashSet<i32> {
    parse_door_config_ids(text, &HashMap::new())
}

/// The [`parse_door_config`] rule with the `scripts/areas/*/configs`
/// header style: `[loc_N]` blocks parse directly and `[name]` blocks
/// resolve through `ids` (the `pack/loc.pack` map), so e.g. the Al Kharid
/// toll gates (`border_gate.loc`'s `[border_gate_toll_left/_right]`,
/// `op1=Open`) join the door set under their own names. Numeric blocks
/// behave exactly as in [`parse_door_config`].
pub fn parse_door_config_ids(text: &str, ids: &HashMap<String, i32>) -> HashSet<i32> {
    let mut door_ids = HashSet::new();
    let mut cur: Option<i32> = None;
    let mut openable = false;
    for raw in text.lines() {
        let line = raw.trim();
        let header =
            loc_header(line).or_else(|| named_loc_header(line).and_then(|n| ids.get(n).copied()));
        if let Some(n) = header {
            if let Some(id) = cur {
                if openable {
                    door_ids.insert(id);
                }
            }
            cur = Some(n);
            openable = false;
        } else if cur.is_some()
            && (line == "op1=Open"
                || line == "category=door_closed"
                || line == "category=gate_main_closed"
                || line == "category=gate_outer_closed")
        {
            openable = true;
        }
    }
    if let Some(id) = cur {
        if openable {
            door_ids.insert(id);
        }
    }
    door_ids
}

/// Closed door loc id → its open leaf id: every `[loc_N]` (or `[name]`,
/// resolved through `ids`) block's `param=next_loc_stage,loc_M` (the id
/// the door changes into when opened).
/// Name-valued params (`param=next_loc_stage,<name>`) resolve through the
/// loc id map; unparseable values carry nothing.
pub fn parse_door_open_ids(text: &str, ids: &HashMap<String, i32>) -> HashMap<i32, i32> {
    let mut out = HashMap::new();
    let mut cur: Option<(i32, Option<i32>)> = None;
    for raw in text.lines() {
        let line = raw.trim();
        let header =
            loc_header(line).or_else(|| named_loc_header(line).and_then(|n| ids.get(n).copied()));
        if let Some(n) = header {
            if let Some((id, Some(open))) = cur {
                out.insert(id, open);
            }
            cur = Some((n, None));
        } else if let Some((_, open)) = cur.as_mut() {
            if let Some(param) = line.strip_prefix("param=") {
                if let Some((key, value)) = param.split_once(',') {
                    if key.trim() == "next_loc_stage" {
                        *open = door_open_value(value.trim(), ids);
                    }
                }
            }
        }
    }
    if let Some((id, Some(open))) = cur {
        out.insert(id, open);
    }
    out
}

/// A `param=next_loc_stage` value → the open leaf id: `loc_N` parses
/// numerically; a bare name resolves through the loc id map.
fn door_open_value(value: &str, ids: &HashMap<String, i32>) -> Option<i32> {
    if let Some(n) = value.strip_prefix("loc_") {
        n.parse().ok()
    } else if let Some(&id) = ids.get(value) {
        Some(id)
    } else {
        None
    }
}

/// Loc ids that do **not** block walk: `[loc_N]` blocks with `blockwalk=no`,
/// `category=door_opened`, or `op1=Close`. Absent `blockwalk` is the 274
/// default (block). Unknown loc ids are treated as blocking by the bake.
pub fn parse_passable_locs(text: &str) -> HashSet<i32> {
    let mut ids = HashSet::new();
    let mut cur: Option<i32> = None;
    let mut passable = false;
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(n) = loc_header(line) {
            if let Some(id) = cur {
                if passable {
                    ids.insert(id);
                }
            }
            cur = Some(n);
            passable = false;
        } else if cur.is_some()
            && (line == "blockwalk=no" || line == "category=door_opened" || line == "op1=Close")
        {
            passable = true;
        }
    }
    if let Some(id) = cur {
        if passable {
            ids.insert(id);
        }
    }
    ids
}

/// `[loc_N]` block header -> `N`.
fn loc_header(line: &str) -> Option<i32> {
    line.strip_prefix("[loc_")?.strip_suffix(']')?.parse().ok()
}

/// `[<name>]` block header -> the name (the `scripts/areas/*/configs`
/// style, e.g. `[border_gate_toll_left]`).
fn named_loc_header(line: &str) -> Option<&str> {
    let name = line.strip_prefix('[')?.strip_suffix(']')?;
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return None;
    }
    Some(name)
}

/// Parse one mapsquare jm2 file (level 0 only). A MAP flag with bit 0 set
/// (`fN`, BLOCK_MAP_SQUARE) is blocked; tiles without a MAP line are not
/// walkable. A LOC whose loc id is in `door_ids` (openable wall doors from
/// the Server door configs) with shape 0 becomes a [`DoorEdge`] crossing the
/// wall: angle 0/2 crosses east-west, angle 1/3 north-south, and the door's
/// own tile is marked not walkable. The edge's `from`/`to` snap to the
/// nearest walkable tile on `collision` (see
/// [`WorldCollision::nearest_walkable`]), not a blind ±1 around the loc.
/// Other blocking locs (unknown types
/// default to blockwalk) stamp their footprint unwalkable, except door
/// from/to tiles. Ground decor, wall decor, and roofs are skipped. Open-door
/// stages in `passable` are not stamped. Malformed lines are skipped. I/O
/// failures and files without a MAP section are errors (callers skip that
/// mapsquare).
pub fn parse_mapsquare_jm2(
    path: &Path,
    mapsquare_x: i32,
    mapsquare_z: i32,
    door_ids: &HashSet<i32>,
    passable: &HashSet<i32>,
    collision: &WorldCollision,
) -> Result<Mapsquare, PackError> {
    let text = std::fs::read_to_string(path).map_err(PackError::Io)?;
    parse_mapsquare_text(
        &text,
        mapsquare_x,
        mapsquare_z,
        door_ids,
        passable,
        collision,
    )
    .ok_or_else(|| PackError::BadLength(format!("{}: no MAP section", path.display())))
}

/// Parse jm2 text into a [`Mapsquare`], or None without a MAP section.
/// Door edge `from`/`to` snap to the nearest walkable tile on `collision`
/// (see [`WorldCollision::nearest_walkable`]), not a blind ±1 around the
/// loc.
fn parse_mapsquare_text(
    text: &str,
    mapsquare_x: i32,
    mapsquare_z: i32,
    door_ids: &HashSet<i32>,
    passable: &HashSet<i32>,
    collision: &WorldCollision,
) -> Option<Mapsquare> {
    let mut walk = vec![0u8; SQUARE * SQUARE];
    let mut locs = Vec::new();
    let mut in_map = false;
    let mut in_loc = false;
    let mut saw_map = false;
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
            if let Some((level, x, z, flags)) = parse_map_line(line) {
                // The 274N grid walk is one level-0 plane; upper-level rows
                // belong to the whole-world collision bake instead.
                if level == 0 {
                    walk[z * SQUARE + x] = if flags & 1 != 0 { 0 } else { 1 };
                }
            }
        } else if in_loc {
            if let Some(loc) = parse_loc_fields(line) {
                if loc.level == 0 {
                    locs.push(loc);
                }
            }
        }
    }
    if !saw_map {
        return None;
    }

    let mut doors = Vec::new();
    let mut door_sides = HashSet::new();
    for loc in &locs {
        if let Some(door) = door_edge(loc, mapsquare_x, mapsquare_z, door_ids, collision) {
            walk[loc.z * SQUARE + loc.x] = 0;
            for side in [door.from, door.to] {
                if let Some((x, z)) = local_in_square(side, mapsquare_x, mapsquare_z) {
                    door_sides.insert((x, z));
                }
            }
            // Bidirectional: the same loc opens either way.
            doors.push(door);
            doors.push(DoorEdge {
                loc: door.loc,
                loc_id: door.loc_id,
                from: door.to,
                to: door.from,
            });
        }
    }
    for loc in &locs {
        if passable.contains(&loc.loc_id) || !loc_blocks_tile(loc.shape) {
            continue;
        }
        if door_sides.contains(&(loc.x, loc.z)) {
            continue;
        }
        walk[loc.z * SQUARE + loc.x] = 0;
    }

    Some(Mapsquare {
        x: mapsquare_x,
        z: mapsquare_z,
        walk,
        doors,
    })
}

/// One bbox [`StepGrid`] on level 0 covering every listed mapsquare. Tiles
/// outside any square are not walkable; squares may leave gaps between them.
pub fn merge_squares(squares: &[Mapsquare]) -> StepGrid {
    assert!(
        !squares.is_empty(),
        "merge_squares needs at least one mapsquare"
    );
    let min_x = squares.iter().map(|s| s.x * SQUARE as i32).min().unwrap();
    let min_z = squares.iter().map(|s| s.z * SQUARE as i32).min().unwrap();
    let max_x = squares
        .iter()
        .map(|s| (s.x + 1) * SQUARE as i32)
        .max()
        .unwrap();
    let max_z = squares
        .iter()
        .map(|s| (s.z + 1) * SQUARE as i32)
        .max()
        .unwrap();
    let (width, height) = ((max_x - min_x) as usize, (max_z - min_z) as usize);
    let mut walk = vec![0u8; width * height];
    let mut doors = Vec::new();
    for sq in squares {
        for z in 0..SQUARE {
            let az = sq.z * SQUARE as i32 + z as i32;
            for x in 0..SQUARE {
                let ax = sq.x * SQUARE as i32 + x as i32;
                walk[(az - min_z) as usize * width + (ax - min_x) as usize] =
                    sq.walk[z * SQUARE + x];
            }
        }
        doors.extend(sq.doors.iter().copied());
    }
    StepGrid::from_parts(
        Tile {
            x: min_x,
            z: min_z,
            level: 0,
        },
        width,
        height,
        walk,
        doors,
    )
}

/// Section header `==== NAME ====`, or None for content lines.
pub(crate) fn section(line: &str) -> Option<&str> {
    line.strip_prefix("==== ")?.strip_suffix(" ====")
}

/// Parse a MAP line into `(level, x, z, flags)`, levels 0..=3 only.
pub(crate) fn parse_map_line(line: &str) -> Option<(i32, usize, usize, u32)> {
    let (coords, rest) = line.split_once(':')?;
    let mut c = coords.split_whitespace();
    let level: i32 = c.next()?.parse().ok()?;
    let x: i32 = c.next()?.parse().ok()?;
    let z: i32 = c.next()?.parse().ok()?;
    if c.next().is_some() {
        return None;
    }
    if !(0..=3).contains(&level) {
        return None;
    }
    let (x, z) = (x as usize, z as usize);
    if x >= SQUARE || z >= SQUARE {
        return None;
    }
    // The raw `fN` flag byte (client `mapl[level][x][z]`): bit 0 is
    // BLOCK, bit 1 is LINK_BELOW. A row with no `f` token carries no flags.
    let flags = rest
        .split_whitespace()
        .find_map(|tok| tok.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()))
        .unwrap_or(0);
    Some((level, x, z, flags))
}

/// One loc placement inside a mapsquare.
pub(crate) struct LocOnSquare {
    pub(crate) level: i32,
    pub(crate) x: usize,
    pub(crate) z: usize,
    pub(crate) loc_id: i32,
    pub(crate) shape: i32,
    pub(crate) angle: i32,
}

/// Walls (0..=3), diagonal wall (9), and centrepiece (10, 11) occupy a walk
/// footprint. Ground decor (22) only blocks when active; wall decor (4..=8)
/// and roofs (12..=21) do not.
fn loc_blocks_tile(shape: i32) -> bool {
    matches!(shape, 0..=3 | 9..=11)
}

/// Parse a LOC line into a placement, levels 0..=3 only.
pub(crate) fn parse_loc_fields(line: &str) -> Option<LocOnSquare> {
    let (coords, rest) = line.split_once(':')?;
    let mut c = coords.split_whitespace();
    let level: i32 = c.next()?.parse().ok()?;
    let x: i32 = c.next()?.parse().ok()?;
    let z: i32 = c.next()?.parse().ok()?;
    if c.next().is_some() {
        return None;
    }
    if !(0..=3).contains(&level) {
        return None;
    }
    let (x, z) = (x as usize, z as usize);
    if x >= SQUARE || z >= SQUARE {
        return None;
    }
    let mut t = rest.split_whitespace();
    let loc_id: i32 = t.next()?.parse().ok()?;
    let shape: i32 = t.next()?.parse().ok()?;
    let angle: i32 = t.next().map_or(Ok(0), |a| a.parse()).ok()?;
    Some(LocOnSquare {
        level,
        x,
        z,
        loc_id,
        shape,
        angle,
    })
}

/// Shape-0 openable wall door -> DoorEdge, or None. `from`/`to` snap to
/// the nearest walkable tile perpendicular to the wall (west/east for an
/// E-W wall, south/north for a N-S wall) on the baked collision: the blind
/// ±1 can land on a wall loc right outside the door, which the router can
/// no longer step onto.
fn door_edge(
    loc: &LocOnSquare,
    mapsquare_x: i32,
    mapsquare_z: i32,
    door_ids: &HashSet<i32>,
    collision: &WorldCollision,
) -> Option<DoorEdge> {
    if !door_ids.contains(&loc.loc_id) || loc.shape != 0 {
        return None;
    }
    let tile = Tile {
        x: mapsquare_x * SQUARE as i32 + loc.x as i32,
        z: mapsquare_z * SQUARE as i32 + loc.z as i32,
        level: 0,
    };
    let door = WorldTile {
        x: tile.x,
        z: tile.z,
        level: 0,
    };
    let snap = |dx: i32, dz: i32| to_tile(collision.nearest_walkable(door, dx, dz));
    let (from, to) = match loc.angle {
        // N-S wall (WEST/EAST facing): cross east-west.
        0 | 2 => (snap(-1, 0), snap(1, 0)),
        // E-W wall (NORTH/SOUTH facing): cross south-north.
        1 | 3 => (snap(0, -1), snap(0, 1)),
        _ => return None,
    };
    Some(DoorEdge {
        loc: tile,
        loc_id: loc.loc_id,
        from,
        to,
    })
}

/// `WorldTile` -> the nav [`Tile`] (both are `x/z/level` triples).
fn to_tile(t: WorldTile) -> Tile {
    Tile {
        x: t.x,
        z: t.z,
        level: t.level,
    }
}

/// Absolute tile -> local mapsquare coords, if it sits in that square.
fn local_in_square(t: Tile, mapsquare_x: i32, mapsquare_z: i32) -> Option<(usize, usize)> {
    if t.level != 0 {
        return None;
    }
    let x = t.x - mapsquare_x * SQUARE as i32;
    let z = t.z - mapsquare_z * SQUARE as i32;
    if x < 0 || z < 0 {
        return None;
    }
    let (x, z) = (x as usize, z as usize);
    if x >= SQUARE || z >= SQUARE {
        return None;
    }
    Some((x, z))
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
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::{
        decode, decode_flags_sidecar, decode_grid, derive_banks, encode, encode_flags_sidecar,
        encode_grid, merge_squares, parse_door_config, parse_door_config_ids, parse_door_open_ids,
        parse_mapsquare_text, parse_passable_locs, walkable_dots, BankAccess, BankStand, Mapsquare,
        SQUARE, VERSION,
    };
    use crate::collision::{derive_walkable, pack_walk, walk_word_from_parts, WorldCollision};
    use crate::grid::StepGrid;
    use crate::pack::PackError;
    use crate::tile::Tile;
    use crate::transport::{DoorDir, TransportEdge, TransportGraph, TransportKind};
    use api::snapshot::WorldTile;
    use client::dash3d::CollisionFlag;

    #[test]
    fn pack_roundtrip_fixture_door() {
        let g = StepGrid::fixture_door_corridor();
        let bytes = encode_grid(&g);
        let h = decode_grid(&bytes).unwrap();
        assert!(h.walkable(Tile {
            x: 0,
            z: 0,
            level: 0
        }));
        assert_eq!(h.doors.len(), g.doors.len());
    }

    #[test]
    fn pack_walk_roundtrips_step_ok_vs_u32_flags() {
        let mut flags = vec![0u32; 4 * 3 * 3];
        flags[1] = CollisionFlag::W_S as u32; // face only
        flags[3] = CollisionFlag::WALK_SCENERY as u32 | CollisionFlag::WR_GRND as u32;
        let (face, blocked) = pack_walk(&flags);
        assert_eq!(face.len(), flags.len());
        for (i, f) in flags.iter().enumerate() {
            let derived = derive_walkable(&[*f])[0];
            let blocked = (blocked[i >> 6] >> (i & 63)) & 1 != 0;
            assert_eq!(walk_word_from_parts(face[i], blocked), derived);
        }
    }

    #[test]
    fn v8_pack_has_no_resident_flags() {
        let flags = vec![0u32; 4 * 2 * 2];
        let (walk, blocked) = pack_walk(&flags);
        let collision = WorldCollision {
            origin: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: 2,
            height: 2,
            walk,
            blocked,
            flags: None,
        };
        let bytes = encode(&collision, &TransportGraph::default(), &[]);
        assert_eq!(bytes[4], VERSION);
        let (c, _, _) = decode(&bytes).unwrap();
        assert!(c.flags.is_none());
        assert_eq!(c.walk.len(), 16);
    }

    #[test]
    fn v8_decode_rejects_v7_and_older() {
        let flags = vec![0u32; 4 * 2 * 2];
        let (walk, blocked) = pack_walk(&flags);
        let collision = WorldCollision {
            origin: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: 2,
            height: 2,
            walk,
            blocked,
            flags: None,
        };
        let mut bytes = encode(&collision, &TransportGraph::default(), &[]);
        bytes[4] = 7;
        assert!(matches!(decode(&bytes), Err(PackError::BadVersion(7))));
        bytes[4] = 6;
        assert!(matches!(decode(&bytes), Err(PackError::BadVersion(6))));
        bytes[4] = 5;
        assert!(matches!(decode(&bytes), Err(PackError::BadVersion(5))));
        bytes[4] = 4;
        assert!(matches!(decode(&bytes), Err(PackError::BadVersion(4))));
    }

    #[test]
    fn flags_sidecar_roundtrips_origin_and_cells() {
        let flags = vec![1u32, 2, 3, 4];
        let origin = WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        };
        let bytes = encode_flags_sidecar(origin, 2, 2, &flags);
        assert_eq!(&bytes[..4], b"274F");
        let (o, w, h, out) = decode_flags_sidecar(&bytes).unwrap();
        assert_eq!(o, origin);
        assert_eq!((w, h), (2, 2));
        assert_eq!(out, flags);
    }

    #[test]
    fn roundtrip_collision_and_transport_graph() {
        let plane = vec![0, 0, 1, 0, 0, 0];
        let mut flags = vec![0u32; 4 * plane.len()];
        flags[..plane.len()].copy_from_slice(&plane);
        // Distinct upper-plane content pins the four-plane wire layout.
        flags[plane.len()..2 * plane.len()].copy_from_slice(&[7; 6]);
        let (walk, blocked) = pack_walk(&flags);
        let collision = WorldCollision {
            origin: WorldTile {
                x: 3200,
                z: 3200,
                level: 0,
            },
            width: 3,
            height: 2,
            walk,
            blocked,
            flags: None,
        };
        let mut graph = TransportGraph::default();
        let door = TransportEdge {
            kind: TransportKind::Door,
            at: WorldTile {
                x: 3201,
                z: 3200,
                level: 0,
            },
            to: WorldTile {
                x: 3203,
                z: 3200,
                level: 0,
            },
            loc_id: 1530,
            option: 1,
            ticks: 1,
            dir: Some(DoorDir::N),
            open_loc_id: Some(1531),
            skill_req: vec![],
            item_req: vec![],
            quest_req: vec![],
            varp_req: vec![],
            worn_req: vec![772], // dramen_staff on the Zanaris shed door
        };
        let ladder = TransportEdge {
            kind: TransportKind::Ladder,
            at: WorldTile {
                x: 3200,
                z: 3200,
                level: 0,
            },
            to: WorldTile {
                x: 3201,
                z: 3201,
                level: 1,
            },
            loc_id: 1747,
            option: 1,
            ticks: 3,
            dir: None,
            open_loc_id: None,
            skill_req: vec![(16, 5)],
            item_req: vec![(995, 10)],
            quest_req: vec!["Restless Ghost".into()],
            varp_req: vec![(4, 1)],
            worn_req: vec![],
        };
        let di = graph.edges.len();
        graph.edges.push(door);
        let li = graph.edges.len();
        graph.edges.push(ladder);
        let glider = TransportEdge {
            kind: TransportKind::Glider,
            at: WorldTile {
                x: 2465,
                z: 3501,
                level: 3,
            },
            to: WorldTile {
                x: 2850,
                z: 3497,
                level: 0,
            },
            loc_id: 170,
            option: 1,
            ticks: 4,
            dir: None,
            open_loc_id: None,
            skill_req: vec![],
            item_req: vec![],
            quest_req: vec![],
            varp_req: vec![(150, 160)],
            worn_req: vec![],
        };
        let gi = graph.edges.len();
        graph.edges.push(glider);
        // A spirit-tree edge (kind 7) and the reserved NPC kind (8) ride
        // the same wire byte without a version bump.
        let spirit = TransportEdge {
            kind: TransportKind::SpiritTree,
            at: WorldTile {
                x: 2460,
                z: 3445,
                level: 0,
            },
            to: WorldTile {
                x: 2542,
                z: 3169,
                level: 0,
            },
            loc_id: 1293,
            option: 1,
            ticks: 1,
            dir: None,
            open_loc_id: None,
            skill_req: vec![],
            item_req: vec![],
            quest_req: vec![],
            varp_req: vec![(150, 160)],
            worn_req: vec![],
        };
        let si = graph.edges.len();
        graph.edges.push(spirit);
        let npc = TransportEdge {
            kind: TransportKind::Npc,
            at: WorldTile {
                x: 2500,
                z: 3500,
                level: 0,
            },
            to: WorldTile {
                x: 2600,
                z: 3400,
                level: 0,
            },
            loc_id: 1,
            option: 1,
            ticks: 2,
            dir: None,
            open_loc_id: None,
            skill_req: vec![],
            item_req: vec![],
            quest_req: vec![],
            varp_req: vec![],
            worn_req: vec![],
        };
        let ni = graph.edges.len();
        graph.edges.push(npc);
        // The any-tile teleport layer (Varrock spell): stored as a kind-4
        // edge in the same array, split back out on decode.
        graph.teleports.push(TransportEdge {
            kind: TransportKind::Teleport,
            at: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            to: WorldTile {
                x: 3213,
                z: 3424,
                level: 0,
            },
            loc_id: 0,
            option: 0,
            ticks: 3,
            dir: None,
            open_loc_id: None,
            skill_req: vec![(6, 25)],
            item_req: vec![(554, 1), (556, 3), (563, 1)],
            quest_req: vec![],
            varp_req: vec![],
            worn_req: vec![],
        });
        graph.at.entry(graph.edges[di].at).or_default().push(di);
        graph.at.entry(graph.edges[li].at).or_default().push(li);
        graph.at.entry(graph.edges[gi].at).or_default().push(gi);
        graph.at.entry(graph.edges[si].at).or_default().push(si);
        graph.at.entry(graph.edges[ni].at).or_default().push(ni);

        let bytes = encode(&collision, &graph, &[]);
        let (c, g, _) = decode(&bytes).unwrap();
        assert_eq!(c.origin, collision.origin);
        assert_eq!(c.width, collision.width);
        assert_eq!(c.height, collision.height);
        assert_eq!(c.walk, collision.walk);
        assert!(c.flags.is_none());
        assert_eq!(g.edges, graph.edges);
        // The door edge's new fields round-trip on the wire.
        assert_eq!(g.edges[di].dir, Some(DoorDir::N));
        assert_eq!(g.edges[di].open_loc_id, Some(1531));
        assert_eq!(g.edges[di].worn_req, vec![772]);
        // The new kinds round-trip on the v4 wire (7 spirit tree, 8 NPC).
        assert_eq!(g.edges[si].kind, TransportKind::SpiritTree);
        assert_eq!(g.edges[si].varp_req, vec![(150, 160)]);
        assert_eq!(g.edges[ni].kind, TransportKind::Npc);
        // Teleports round-trip in their own layer, and the at-index is
        // rebuilt from the ordinary edges only.
        assert_eq!(g.teleports, graph.teleports);
        assert_eq!(g.at, graph.at);
        assert!(!g.at.contains_key(&WorldTile {
            x: 0,
            z: 0,
            level: 0
        }));
        // The two formats do not cross-decode: the grid rejects pack magic
        // and vice versa.
        assert!(matches!(decode_grid(&bytes), Err(PackError::BadMagic)));
        assert!(matches!(
            decode(&encode_grid(&StepGrid::fixture_open_3x3())),
            Err(PackError::BadMagic)
        ));
    }

    #[test]
    fn decode_rejects_old_version_streams() {
        // A version-2 or version-3 stream (pre-four-plane wire) is
        // rejected, not mis-read: the re-bake immediately rewrites it at
        // the current version. Versions 4, 5, and 6 are rejected too (see
        // `v8_decode_rejects_v7_and_older`).
        let plane = vec![0, 0, 1, 0, 0, 0];
        let mut flags = vec![0u32; 4 * plane.len()];
        flags[..plane.len()].copy_from_slice(&plane);
        let (walk, blocked) = pack_walk(&flags);
        let collision = WorldCollision {
            origin: WorldTile {
                x: 3200,
                z: 3200,
                level: 0,
            },
            width: 3,
            height: 2,
            walk,
            blocked,
            flags: None,
        };
        let graph = TransportGraph::default();
        let mut bytes = encode(&collision, &graph, &[]);
        // The version byte sits right after the 4-byte magic.
        bytes[4] = 3;
        assert!(matches!(decode(&bytes), Err(PackError::BadVersion(3))));
        bytes[4] = 2;
        assert!(matches!(decode(&bytes), Err(PackError::BadVersion(2))));
    }

    #[test]
    fn v8_roundtrips_worn_req() {
        // v8 carries the fifth per-edge req list (the worn-item ids) on
        // the packed-walk wire; no pre-v8 stream decodes.
        let plane = vec![0, 0, 1, 0, 0, 0];
        let mut flags = vec![0u32; 4 * plane.len()];
        flags[..plane.len()].copy_from_slice(&plane);
        let (walk, blocked) = pack_walk(&flags);
        let collision = WorldCollision {
            origin: WorldTile {
                x: 3200,
                z: 3200,
                level: 0,
            },
            width: 3,
            height: 2,
            walk,
            blocked,
            flags: None,
        };
        let door = TransportEdge {
            kind: TransportKind::Door,
            at: WorldTile {
                x: 3201,
                z: 3200,
                level: 0,
            },
            to: WorldTile {
                x: 3203,
                z: 3200,
                level: 0,
            },
            loc_id: 2406,
            option: 1,
            ticks: 1,
            dir: Some(DoorDir::N),
            open_loc_id: Some(1532),
            skill_req: vec![],
            item_req: vec![],
            quest_req: vec!["Lost City".into()],
            varp_req: vec![],
            worn_req: vec![772],
        };
        let mut graph = TransportGraph::default();
        graph.edges.push(door.clone());
        graph.at.entry(door.at).or_default().push(0);
        let bytes = encode(&collision, &graph, &[]);
        // The version byte sits right after the 4-byte magic: v8 now.
        assert_eq!(bytes[4], VERSION);
        let (c, g, _) = decode(&bytes).unwrap();
        assert_eq!(g.edges, graph.edges);
        assert_eq!(g.edges[0].worn_req, vec![772]);
        assert_eq!(g.at, graph.at);
        assert_eq!(c.walk, collision.walk);
        assert!(c.flags.is_none());
    }

    #[test]
    fn decode_grid_rejects_bad_magic() {
        assert!(matches!(decode_grid(b"XXXX"), Err(PackError::BadMagic)));
    }

    #[test]
    fn decode_grid_rejects_truncated_pack() {
        let bytes = encode_grid(&StepGrid::fixture_door_corridor());
        assert!(matches!(
            decode_grid(&bytes[..bytes.len() - 1]),
            Err(PackError::Truncated)
        ));
    }

    #[test]
    fn decode_grid_rejects_oversized_grid() {
        // Huge width would try to allocate GiB of walk bytes.
        let bytes = header(0, u32::MAX, 1);
        assert!(matches!(decode_grid(&bytes), Err(PackError::BadLength(_))));
    }

    #[test]
    fn decode_grid_rejects_zero_grid() {
        assert!(matches!(
            decode_grid(&header(0, 0, 1)),
            Err(PackError::BadLength(_))
        ));
        assert!(matches!(
            decode_grid(&header(0, 1, 0)),
            Err(PackError::BadLength(_))
        ));
    }

    /// Magic + version + zero origin + width/height, nothing else.
    fn header(level: i32, width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"274N");
        bytes.push(1);
        for v in [0i32, 0, level] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        bytes.extend_from_slice(&width.to_le_bytes());
        bytes.extend_from_slice(&height.to_le_bytes());
        bytes
    }

    #[test]
    fn walkable_dots_lists_only_walkable_tiles() {
        let g = StepGrid::fixture_door_corridor();
        let dots: Vec<Tile> = walkable_dots(&g, 0).collect();
        assert_eq!(dots.len(), 4);
        assert!(!dots.contains(&Tile {
            x: 2,
            z: 0,
            level: 0
        }));
        assert_eq!(walkable_dots(&g, 1).count(), 0);
    }

    #[test]
    fn parse_door_config_collects_openable_doors() {
        // Mirrors the real config format: closed/open counterpart blocks.
        let text = "\
[loc_1512]
name=Large door
op1=Open
category=door_closed

[loc_1513]
op1=Open

[loc_1514]
op1=Close
category=door_opened

[loc_1530]
op1=Open
category=door_closed

[membergatel]
name=Gate
op1=Open
";
        let ids = parse_door_config(text);
        assert!(ids.contains(&1512));
        assert!(ids.contains(&1513));
        assert!(ids.contains(&1530));
        assert!(!ids.contains(&1514));
        assert!(!ids.contains(&1531));
    }

    #[test]
    fn parse_door_config_ids_resolves_named_blocks() {
        // Mirrors `scripts/areas/area_alkharid/configs/border_gate.loc`:
        // name-keyed blocks that `parse_door_config` (numeric-only) skips.
        let text = "\
[border_gate_toll_left]
name=Gate
op1=Open
category=border_gate_toll_left
param=next_loc_stage,loc_1562

[border_gate_toll_right]
name=Gate
op1=Open
category=border_gate_toll_right
param=next_loc_stage,loc_1563
";
        let mut ids = HashMap::new();
        ids.insert("border_gate_toll_left".to_string(), 2882);
        ids.insert("border_gate_toll_right".to_string(), 2883);
        ids.insert("loc_1562".to_string(), 1562);
        ids.insert("loc_1563".to_string(), 1563);
        let doors = parse_door_config_ids(text, &ids);
        assert!(doors.contains(&2882));
        assert!(doors.contains(&2883));
        // The numeric-only view still ignores the name-keyed blocks.
        assert!(!parse_door_config(text).contains(&2882));
        // The open-leaf params resolve under the named blocks too.
        let open = parse_door_open_ids(text, &ids);
        assert_eq!(open.get(&2882), Some(&1562));
        assert_eq!(open.get(&2883), Some(&1563));
    }

    #[test]
    fn parse_door_config_collects_gate_closed_categories() {
        // Mirrors gates.loc: closed/open counterpart blocks carrying the
        // fence-gate categories. `gate_main_closed` / `gate_outer_closed`
        // are openable like `door_closed`; the `*_open` counterpart states
        // (`op1=Close`) are not.
        let text = "\
[loc_1551]
name=Gate
op1=Open
category=gate_main_closed

[loc_1552]
op1=Close
category=gate_main_open

[loc_1553]
op1=Open
category=gate_outer_closed
";
        let ids = parse_door_config(text);
        assert!(ids.contains(&1551));
        assert!(!ids.contains(&1552));
        assert!(ids.contains(&1553));
    }

    #[test]
    fn parse_door_open_ids_reads_next_loc_stage() {
        let text = "\
[loc_1530]
name=Door
op1=Open
category=door_closed
param=next_loc_stage,loc_1531

[loc_1512]
op1=Open
param=next_loc_stage,loc_1513

[loc_1514]
op1=Open
param=next_loc_stage,elenagateopen

[membergatel]
name=Gate
op1=Open
";
        // Numeric `loc_N` values parse directly; the name-valued one
        // resolves through the loc id map.
        let ids = {
            let mut m = std::collections::HashMap::new();
            m.insert("elenagateopen".to_string(), 1535);
            m
        };
        let open = parse_door_open_ids(text, &ids);
        assert_eq!(open.get(&1530), Some(&1531));
        assert_eq!(open.get(&1512), Some(&1513));
        assert_eq!(open.get(&1514), Some(&1535));
        // Non-numeric headers and unknown names carry nothing.
        assert_eq!(open.get(&1534), None);
    }

    /// A 64×64 level-0 collision at the given mapsquare with the given
    /// local `(x, z, flag)` stamps, everything else walkable — the door
    /// snap's view of the world (mirrors the flags `bake_from_maps` stamps
    /// for the fixture's MAP/LOC lines; `V_*` range flags are omitted since
    /// they are not in the walk mask).
    fn square_collision(mx: i32, mz: i32, extras: &[(usize, usize, u32)]) -> WorldCollision {
        let mut flags = vec![0u32; SQUARE * SQUARE];
        for &(x, z, f) in extras {
            flags[z * SQUARE + x] |= f;
        }
        let (walk, blocked) = pack_walk(&flags);
        WorldCollision {
            origin: WorldTile {
                x: mx * SQUARE as i32,
                z: mz * SQUARE as i32,
                level: 0,
            },
            width: SQUARE,
            height: SQUARE,
            walk,
            blocked,
            flags: None,
        }
    }

    #[test]
    fn parse_jm2_text_pins_catherby_door() {
        let door_ids = parse_door_config(
            "[loc_1530]\nop1=Open\ncategory=door_closed\n[loc_980]\nop1=Close\ncategory=door_opened\n",
        );
        let text = "\
==== MAP ====
0 0 0: h1 o6 u48
0 1 0: f1 u48
0 0 1: f16 u50
0 0 46: h1 o6 f1 u50
==== LOC ====
0 0 46: 1530 0 1
0 1 46: 980 0 0
1 0 46: 1530 0 1
0 0 47: 1530 0 7
==== NPC ====
0 0 0: 1234
";
        // The bake for this text: (0,46) carries MAP f1 (WR_GRND) plus the
        // door's W_N and wall 980's W_E; (0,47) the door's W_S face stamp.
        let collision = square_collision(
            44,
            53,
            &[
                (1, 0, CollisionFlag::WR_GRND as u32),
                (
                    0,
                    46,
                    CollisionFlag::WR_GRND as u32
                        | CollisionFlag::W_N as u32
                        | CollisionFlag::W_E as u32,
                ),
                (1, 46, CollisionFlag::W_W as u32),
                (0, 47, CollisionFlag::W_S as u32),
            ],
        );
        let sq =
            parse_mapsquare_text(text, 44, 53, &door_ids, &HashSet::new(), &collision).unwrap();
        // (0,0): no f flag -> walkable; (1,0): f1 bit 0 -> blocked;
        // (0,1): f16 bit 0 clear -> walkable; (2,0): no MAP line -> blocked.
        assert_eq!(sq.walk[0], 1);
        assert_eq!(sq.walk[1], 0);
        assert_eq!(sq.walk[64], 1);
        assert_eq!(sq.walk[2], 0);
        // The Catherby closed door: 1530 @ local (0,46) -> 2816,3438,0,
        // both from→to and to→from (same loc, loc_id). The north side
        // snaps past (0,47) — the door's own blocked south-face stamp — to
        // the next walkable tile.
        assert_eq!(sq.doors.len(), 2);
        let south = Tile {
            x: 2816,
            z: 3437,
            level: 0,
        };
        let north = Tile {
            x: 2816,
            z: 3440,
            level: 0,
        };
        let loc = Tile {
            x: 2816,
            z: 3438,
            level: 0,
        };
        let fwd = sq
            .doors
            .iter()
            .find(|d| d.from == south && d.to == north)
            .expect("Catherby south→north door");
        let rev = sq
            .doors
            .iter()
            .find(|d| d.from == north && d.to == south)
            .expect("Catherby reverse neighbour");
        assert_eq!(fwd.loc, loc);
        assert_eq!(fwd.loc_id, 1530);
        assert_eq!(rev.loc, loc);
        assert_eq!(rev.loc_id, 1530);
        // The door tile is a wall: not walkable.
        assert_eq!(sq.walk[46 * 64], 0);
    }

    #[test]
    fn parse_passable_locs_blockwalk_no_and_open_door() {
        let text = "\
[loc_980]
name=Fence
[loc_1124]
blockwalk=no
[loc_1531]
op1=Close
category=door_opened
[loc_1259]
blockwalk=yes
";
        let ids = parse_passable_locs(text);
        assert!(ids.contains(&1124));
        assert!(ids.contains(&1531));
        assert!(!ids.contains(&980));
        assert!(!ids.contains(&1259));
    }

    #[test]
    fn parse_jm2_blocking_loc_marks_tile_unwalkable() {
        // loc 980 (fencing) is unknown / default-block; local (0,45) of
        // mapsquare 44,53 is absolute 2816,3437.
        let text = "\
==== MAP ====
0 0 45: h1 o6 u50
==== LOC ====
0 0 45: 980 0 0
";
        let collision = square_collision(44, 53, &[]);
        let sq = parse_mapsquare_text(text, 44, 53, &HashSet::new(), &HashSet::new(), &collision)
            .unwrap();
        let grid = merge_squares(&[sq]);
        assert!(!grid.walkable(Tile {
            x: 2816,
            z: 3437,
            level: 0
        }));
    }

    #[test]
    fn parse_jm2_snaps_door_from_to_past_a_wall_loc() {
        let door_ids = parse_door_config("[loc_1530]\nop1=Open\ncategory=door_closed\n");
        let text = "\
==== MAP ====
0 0 45: h1 o6 u50
0 0 46: h1 o6 u50
0 0 47: h1 o6 u50
==== LOC ====
0 0 46: 1530 0 1
0 0 45: 980 0 0
";
        // Wall 980 right outside the door's south side: the door from/to
        // snap past it to (2816,3436)/(2816,3440) instead of landing on
        // the wall tile, which stays blocked by the loc.
        let collision = square_collision(
            44,
            53,
            &[
                (0, 45, CollisionFlag::W_W as u32),
                (0, 46, CollisionFlag::W_N as u32),
                (0, 47, CollisionFlag::W_S as u32),
            ],
        );
        let sq =
            parse_mapsquare_text(text, 44, 53, &door_ids, &HashSet::new(), &collision).unwrap();
        assert_eq!(sq.doors.len(), 2);
        let south = Tile {
            x: 2816,
            z: 3436,
            level: 0,
        };
        let north = Tile {
            x: 2816,
            z: 3440,
            level: 0,
        };
        assert!(sq.doors.iter().any(|d| d.from == south && d.to == north));
        assert!(sq.doors.iter().any(|d| d.from == north && d.to == south));
        let grid = merge_squares(&[sq]);
        assert!(!grid.walkable(Tile {
            x: 2816,
            z: 3438,
            level: 0
        }));
        assert!(!grid.walkable(Tile {
            x: 2816,
            z: 3437,
            level: 0
        }));
        assert!(grid.walkable(Tile {
            x: 2816,
            z: 3439,
            level: 0
        }));
    }

    #[test]
    fn merge_squares_builds_one_bbox_level0_grid() {
        let a = one_tile_square(50, 50);
        let b = one_tile_square(52, 52);
        let grid = merge_squares(&[a, b]);
        assert_eq!(grid.doors.len(), 0);
        assert!(grid.walkable(Tile {
            x: 3200,
            z: 3200,
            level: 0
        }));
        assert!(grid.walkable(Tile {
            x: 3328,
            z: 3328,
            level: 0
        }));
        // The square gap between the two squares stays blocked.
        assert!(!grid.walkable(Tile {
            x: 3264,
            z: 3264,
            level: 0
        }));
        assert!(!grid.walkable(Tile {
            x: 2816,
            z: 3200,
            level: 0
        }));
    }

    /// A 64×64 square with only its local (0,0) tile walkable.
    fn one_tile_square(x: i32, z: i32) -> Mapsquare {
        let mut walk = vec![0u8; 64 * 64];
        walk[0] = 1;
        Mapsquare {
            x,
            z,
            walk,
            doors: vec![],
        }
    }

    #[test]
    fn v8_roundtrips_bank_stands() {
        // The v8 wire carries the bank stand table after the transport
        // edges: a booth round-trips its name, tile, and the Use-quickly
        // op; the NPC variant round-trips its npc name, op, and the
        // optional dialog choice.
        let flags = vec![0u32; 4 * 2 * 2];
        let (walk, blocked) = pack_walk(&flags);
        let collision = WorldCollision {
            origin: WorldTile {
                x: 3200,
                z: 3200,
                level: 0,
            },
            width: 2,
            height: 2,
            walk,
            blocked,
            flags: None,
        };
        let banks = vec![
            BankStand {
                name: "Bank booth".into(),
                tile: WorldTile {
                    x: 3205,
                    z: 3441,
                    level: 0,
                },
                access: BankAccess::Booth { op: 2 },
            },
            BankStand {
                name: "Banker".into(),
                tile: WorldTile {
                    x: 2810,
                    z: 3445,
                    level: 0,
                },
                access: BankAccess::Npc {
                    name: "shilobanker".into(),
                    op: 3,
                    choose: Some("I'd like to access my bank account, please.".into()),
                },
            },
        ];
        let bytes = encode(&collision, &TransportGraph::default(), &banks);
        assert_eq!(bytes[4], VERSION);
        let (c, g, out) = decode(&bytes).unwrap();
        assert_eq!(out, banks);
        assert_eq!(c.walk, collision.walk);
        assert!(g.edges.is_empty());
    }

    #[test]
    fn bake_emits_bankbooth_use_quickly_only() {
        // The bake derives booth stands from the same content loc pass as
        // the collision bake: `bankbooth` placements join the table with
        // the Use-quickly op (2); the closed booth (`bankboothclosed`) and
        // the tutorial `newbiebankbooth` never do.
        let dir = std::env::temp_dir().join(format!("274bot-nav-banks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for sub in ["", "pack", "scripts/interface_bank/configs", "maps"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        std::fs::write(
            dir.join("pack/loc.pack"),
            "2213=bankbooth\n2215=bankboothclosed\n3045=newbiebankbooth\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("scripts/interface_bank/configs/bank_booth.loc"),
            "[bankbooth]\nname=Bank booth\nop2=Use-quickly\n\n[bankboothclosed]\nname=Closed bank booth\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("maps/m50_50.jm2"),
            "==== MAP ====\n0 0 0: h1 o6 u48\n==== LOC ====\n0 12 32: 2213 10 1\n0 13 32: 2215 10 1\n0 14 32: 3045 10 1\n",
        )
        .unwrap();
        let banks = derive_banks(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            banks.len(),
            1,
            "only the bankbooth placement becomes a stand"
        );
        assert_eq!(banks[0].name, "Bank booth");
        assert_eq!(
            banks[0].tile,
            WorldTile {
                x: 50 * 64 + 12,
                z: 50 * 64 + 32,
                level: 0,
            }
        );
        assert_eq!(banks[0].access, BankAccess::Booth { op: 2 });
    }
}
