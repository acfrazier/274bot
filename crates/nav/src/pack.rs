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
//! Pack format (274V): magic `b"274V"`, version `u8` 9, collision origin
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
//! [`derive_banks`]. [`decode`] accepts version 9 only — a v8 stream (or
//! any earlier one, the v6 packed u16 words included) is
//! [`PackError::BadVersion`];
//! there is no flags→walk compat load. The 274N grid decoder stays for
//! old `.navpack` files; `nav-pack` now writes v9.

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

mod config_parse;
pub use config_parse::{
    parse_door_config, parse_door_config_ids, parse_door_open_ids, parse_passable_locs,
};

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
/// kind byte — no version bump. v9 — the current wire — appends a
/// per-edge `members_req` `u8` (`0`/`1`) after `worn_req`. [`decode`]
/// accepts version 9 only; 8, 7, 6, 5, and older streams are rejected
/// rather than compat-loaded.
/// Rebake with `nav-pack` over `$ENGINE_DIR/../content/maps` whenever the
/// Server content changes (new loc/NPC placements, pack bumps).
const VERSION: u8 = 9;
/// Current pack file magic.
const MAGIC: &[u8; 4] = b"274V";
/// Flags sidecar format version.
const VERSION_FLAGS: u8 = 1;
/// Flags sidecar magic.
const MAGIC_FLAGS: &[u8; 4] = b"274F";
/// Paint-reach sidecar format version.
const VERSION_REACH: u8 = 1;
/// Paint-reach sidecar magic.
const MAGIC_REACH: &[u8; 4] = b"274R";
/// Static canlight sidecar format version.
const VERSION_CANLIGHT: u8 = 1;
/// Static canlight sidecar magic.
const MAGIC_CANLIGHT: &[u8; 4] = b"274L";
/// Pack format identity as it appears in bundled navigation identities: the
/// file magic followed by the format version (`274V9` for the current wire).
/// A format improvement changes this identity and therefore invalidates
/// staged build artifacts.
pub const FORMAT_ID: &str = "274V9";
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
        out.push(if e.members_req { 1 } else { 0 });
    }
    write_bank_stands(&mut out, banks);
    out
}

/// Deserialize the whole-world pack, validating magic, version, and
/// lengths. The `at` index is rebuilt from the decoded edges; kind-4
/// (teleport) edges split back into [`TransportGraph::teleports`] and are
/// excluded from it. Version 9 is the only accepted wire: the collision
/// decodes as the `u8` face bytes plus the packed `SQ_BLOCKED`
/// bit-plane with no resident flags (`flags` is
/// `None` until the sidecar is loaded), and the trailing bank stand table
/// (see [`BankStand`]) decodes after the edges; any other version — 8, 7, 6,
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
///
/// After the header is validated, the already length-checked trailing
/// payload is bulk-converted with little-endian `u32` interpretation —
/// not a per-word [`Cursor`] `read_exact` loop — so large sidecars do not
/// stall startup on scalar decode overhead.
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
    let payload = &bytes[r.position() as usize..];
    if !payload.len().is_multiple_of(4) {
        return Err(PackError::Truncated);
    }
    Ok((origin, width, height, decode_u32le_words(payload)))
}

/// Bulk little-endian `u32` words from a length-checked payload (`len % 4 == 0`).
fn decode_u32le_words(payload: &[u8]) -> Vec<u32> {
    debug_assert!(payload.len().is_multiple_of(4));
    let n = payload.len() / 4;
    let mut flags = Vec::with_capacity(n);
    for chunk in payload.as_chunks::<4>().0 {
        // as_chunks guarantees 4 bytes; avoid Cursor/read_exact per word.
        flags.push(u32::from_le_bytes(*chunk));
    }
    flags
}

/// Decoded paint-reach sidecar: geometry, word count, pack-identity binding
/// and the `bake_reach` bitset. Geometry alone is not identity — the binding
/// is the SHA-256 of the pack that produced the bits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReachSidecar {
    pub origin: WorldTile,
    pub width: usize,
    pub height: usize,
    pub word_count: usize,
    pub binding: [u8; 32],
    pub bits: Vec<u64>,
}

/// Hex form of a 32-byte SHA-256 (lowercase), matching [`crate::manifest::hash_bytes`].
pub fn sha256_hex(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

/// Parse a 64-char lowercase-or-mixed SHA-256 hex digest into 32 bytes.
pub fn sha256_from_hex(hex: &str) -> Result<[u8; 32], String> {
    if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("is not a SHA-256 hex digest".into());
    }
    let mut out = [0u8; 32];
    for (i, chunk) in hex.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let text = std::str::from_utf8(chunk).map_err(|_| "is not a SHA-256 hex digest")?;
        out[i] = u8::from_str_radix(text, 16).map_err(|_| "is not a SHA-256 hex digest")?;
    }
    Ok(out)
}

/// Serialize a paint-reach bitset: magic `b"274R"`, version 1, the same
/// origin/width/height header as the pack, explicit word count, the 32-byte
/// pack SHA-256 binding, then `word_count` little-endian `u64` words.
pub fn encode_reach_sidecar(
    origin: WorldTile,
    width: usize,
    height: usize,
    bits: &[u64],
    binding: &[u8; 32],
) -> Vec<u8> {
    encode_bitset_sidecar(
        MAGIC_REACH,
        VERSION_REACH,
        origin,
        width,
        height,
        bits,
        binding,
    )
}

/// Deserialize a paint-reach sidecar, validating magic, version, grid
/// header, word count and that the trailing payload is exactly that many
/// `u64le` words. Binding bytes are not interpreted here — the caller
/// compares them to the pack identity.
pub fn decode_reach_sidecar(bytes: &[u8]) -> Result<ReachSidecar, PackError> {
    decode_bitset_sidecar(bytes, MAGIC_REACH, VERSION_REACH)
}

/// Decoded static canlight sidecar. Same geometry as [`ReachSidecar`]; the
/// binding is pack+policy identity, not pack SHA alone.
pub type CanlightSidecar = ReachSidecar;

/// Serialize a static canlight bitset: magic `b"274L"`, version 1, the same
/// header dialect as [`encode_reach_sidecar`].
pub fn encode_canlight_sidecar(
    origin: WorldTile,
    width: usize,
    height: usize,
    bits: &[u64],
    binding: &[u8; 32],
) -> Vec<u8> {
    encode_bitset_sidecar(
        MAGIC_CANLIGHT,
        VERSION_CANLIGHT,
        origin,
        width,
        height,
        bits,
        binding,
    )
}

/// Deserialize a static canlight sidecar. Binding bytes are not interpreted
/// here — the caller compares them to pack+policy identity.
pub fn decode_canlight_sidecar(bytes: &[u8]) -> Result<CanlightSidecar, PackError> {
    decode_bitset_sidecar(bytes, MAGIC_CANLIGHT, VERSION_CANLIGHT)
}

fn encode_bitset_sidecar(
    magic: &[u8; 4],
    version: u8,
    origin: WorldTile,
    width: usize,
    height: usize,
    bits: &[u64],
    binding: &[u8; 32],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + 1 + 12 + 8 + 4 + 32 + bits.len() * 8);
    out.extend_from_slice(magic);
    out.push(version);
    for v in [origin.x, origin.z, origin.level] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&(width as u32).to_le_bytes());
    out.extend_from_slice(&(height as u32).to_le_bytes());
    out.extend_from_slice(&(bits.len() as u32).to_le_bytes());
    out.extend_from_slice(binding);
    for word in bits {
        out.extend_from_slice(&word.to_le_bytes());
    }
    out
}

fn decode_bitset_sidecar(
    bytes: &[u8],
    expected_magic: &[u8; 4],
    expected_version: u8,
) -> Result<ReachSidecar, PackError> {
    let mut r = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    r.read_exact(&mut magic).map_err(|_| PackError::Truncated)?;
    if &magic != expected_magic {
        return Err(PackError::BadMagic);
    }
    let mut version = [0u8; 1];
    r.read_exact(&mut version)
        .map_err(|_| PackError::Truncated)?;
    if version[0] != expected_version {
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
    let word_count = read_u32(&mut r)? as usize;
    let mut binding = [0u8; 32];
    r.read_exact(&mut binding)
        .map_err(|_| PackError::Truncated)?;
    let payload = &bytes[r.position() as usize..];
    if payload.len() != word_count.saturating_mul(8) {
        return Err(PackError::Truncated);
    }
    Ok(ReachSidecar {
        origin,
        width,
        height,
        word_count,
        binding,
        bits: decode_u64le_words(payload),
    })
}

/// Bulk little-endian `u64` words from a length-checked payload (`len % 8 == 0`).
fn decode_u64le_words(payload: &[u8]) -> Vec<u64> {
    debug_assert!(payload.len().is_multiple_of(8));
    payload
        .as_chunks::<8>()
        .0
        .iter()
        .map(|chunk| u64::from_le_bytes(*chunk))
        .collect()
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
#[path = "pack_tests.rs"]
mod tests;
