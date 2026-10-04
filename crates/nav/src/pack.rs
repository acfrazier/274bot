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
//! Pack format (274V): magic `b"274V"`, version `u8` 16, the quest-family
//! binding (`u8` `0` = the bake consumed no quest family, `1` = bound, then
//! the family artifact's 32-byte `quest_facts_sha256` and its
//! `quest_extractor_schema` as a nonzero u16le), collision origin
//! `(x, z, level)` i32le, width/height u32le, the [`WorldCollision`]
//! packed walk surface — first the `u8` face byte per tile per level,
//! four planes, level-major, each `width × height` (row-major z then x),
//! then the `SQ_BLOCKED` bit-plane as `u64le` words, 64 cells per word,
//! same indexing — then the transport edge count u32le and per edge
//! `(kind u8, at x/z/level, to x/z/level, loc_id, option, ticks, dir u8,
//! open_loc_id)` i32le plus requirement vectors, membership and wilderness
//! caps, quest-stage gates, the conjunctive `worn_all_req` list, and a
//! per-edge takeoff tag (`0` absent, `1` + exact `(x, z, level)` i32le).
//! The takeoff is stored after quest gates and before approach geometry.
//! Version 14 reserves bit 7 of the existing kind byte for a player-relative
//! ladder/stairs landing. The low seven bits retain the kind; flagged `to - at`
//! encodes the content displacement, resolved against each actual takeoff
//! stand. Absolute edge records are unchanged and the flag adds no wire bytes.
//! Teleport edges round-trip inside the same array as kind-4 edges; [`decode`]
//! splits them out and never indexes them into `at`. After edges come bank
//! stands, Wilderness rules, and the v12 zone section:
//! a kind table (npc id i32, vislevel u16, AP/visibility flags u8, then id
//! and label as length-prefixed UTF-8), zone rows (tag 0 for NPC spawn/radius
//! and tag 1 for a hazard rectangle), curated groups (identity, label, rect,
//! optional level, and zone indices), carves, and shaped-zone masks
//! (zone index u16, north extent u8, and u64 row-major cell bits; shaped NPC
//! rows store the east extent in `r`). Decode rebuilds the validated
//! 8×8 zone index and recomputes Wilderness overlap; neither is on the wire.
//! Every decoded v16 stream has `Some(ZoneTable)`, even when every row count
//! is zero; legacy grids and synthetic in-memory graphs use `zones: None`.
//! Zone counts/indices are bounded to the packed namespaces; malformed rows
//! return [`PackError::BadLength`]. A v15 or older whole-world stream is
//! [`PackError::BadVersion`], never compat-loaded. Takeoff levels must match
//! the loc anchor's valid game plane, and exact tiles must be standable in
//! the decoded collision.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::io::{self, BufRead, Cursor, Read};
use std::num::NonZeroU16;
use std::path::Path;

use api::query::loc_approach::LocApproach;
use api::selected::{FactKey, FactStrings, InclusiveRange, QuestGate, StageWindow};
use api::snapshot::WorldTile;

use crate::collision::WorldCollision;
use crate::grid::{DoorEdge, StepGrid};
use crate::quest_gates::{QuestFamilyId, QuestGates};
use crate::tile::Tile;
use crate::transport::{DoorDir, TransportEdge, TransportGraph, TransportKind};

mod banks;
mod config_parse;
mod mapsquare;
mod sidecars;
mod zones;
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
    encode_canlight_sidecar, encode_flags_sidecar, encode_reach_sidecar, read_canlight_sidecar,
    read_flags_sidecar, read_reach_sidecar, read_reach_sidecar_header, sha256_from_hex, sha256_hex,
    CanlightSidecar, CanlightSidecarLoad, FlagsSidecarLoad, ReachSidecar, ReachSidecarHeader,
    ReachSidecarLoad, FLAGS_HEADER_LEN,
};
#[cfg(test)]
use sidecars::{MAGIC_FLAGS, VERSION_FLAGS};

/// Grid (274N) format version: boolean walk bytes + doors.
const VERSION_GRID: u8 = 1;
/// Grid (274N) file magic.
const MAGIC_GRID: &[u8; 4] = b"274N";
/// Current pack format version (collision + transport graph). v3 adds the
/// per-edge `dir`/`open_loc_id` fields, v4 stores the four collision planes,
/// v5 adds the per-edge worn-item id list `worn_req`, and v6 replaces the
/// four u32 flag planes with compact packed walk words. v7 splits those
/// words into the `u8` face byte plus packed `SQ_BLOCKED` bit-plane. v8
/// appends the bank stand table; v9 adds `members_req`; v10 adds wilderness
/// teleport caps and the Wilderness-level formula. v11 binds the selected
/// quest family and appends typed quest-stage gates. v12 appends the zone
/// table, v13 approach geometry, v14 player-relative landings, v15 consumed
/// resources and returns, and v16 `worn_all_req` plus optional exact takeoff.
pub const VERSION: u8 = 16;
/// Current pack file magic.
const MAGIC: &[u8; 4] = b"274V";
/// Pack format identity as it appears in bundled navigation identities.
/// A format improvement changes this identity and invalidates staged builds.
pub const FORMAT_ID: &str = "274V16";
/// Bytes per door entry.
const DOOR_BYTES: usize = 40;
/// High bit of a ladder/stairs kind byte: `to - at` is a player displacement.
const PLAYER_RELATIVE: u8 = 0x80;
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
            PackError::BadVersion(v) => {
                write!(f, "unsupported pack version {v}; rebake it with nav-pack")
            }
            PackError::Truncated => write!(f, "pack file truncated"),
            PackError::BadLength(m) => write!(f, "inconsistent pack: {m}"),
        }
    }
}

impl std::error::Error for PackError {}

/// Bounded input shared by the byte-slice and streaming decoders.
pub(super) trait PackRead {
    fn remaining(&self) -> usize;
    fn read_bytes_exact(&mut self, buf: &mut [u8]) -> Result<(), PackError>;

    /// Stream keys are borrowed while contiguous in the buffer; a crossing
    /// key needs one temporary allocation. Cursor keys keep their borrowed
    /// interning fast path.
    fn read_fact_key(&mut self, keys: &mut FactStrings) -> Result<FactKey, PackError>
    where
        Self: Sized,
    {
        let len = read_u32(self)? as usize;
        if len > self.remaining() {
            return Err(PackError::Truncated);
        }
        let mut bytes = vec![0; len];
        self.read_bytes_exact(&mut bytes)?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| PackError::BadLength("quest gate key is not UTF-8".into()))?;
        Ok(FactKey(keys.intern(text)))
    }
}

fn map_stream_error(error: io::Error) -> PackError {
    if error.kind() == io::ErrorKind::UnexpectedEof {
        PackError::Truncated
    } else {
        PackError::Io(error)
    }
}

impl PackRead for Cursor<&[u8]> {
    fn remaining(&self) -> usize {
        self.get_ref()
            .len()
            .saturating_sub(self.position() as usize)
    }

    fn read_bytes_exact(&mut self, buf: &mut [u8]) -> Result<(), PackError> {
        Read::read_exact(self, buf).map_err(|_| PackError::Truncated)
    }

    fn read_fact_key(&mut self, keys: &mut FactStrings) -> Result<FactKey, PackError> {
        let len = read_u32(self)? as usize;
        let start = self.position() as usize;
        let end = start.checked_add(len).ok_or(PackError::Truncated)?;
        let text = self.get_ref().get(start..end).ok_or(PackError::Truncated)?;
        let text = std::str::from_utf8(text)
            .map_err(|_| PackError::BadLength("quest gate key is not UTF-8".into()))?;
        self.set_position(end as u64);
        Ok(FactKey(keys.intern(text)))
    }
}

impl<T: PackRead> PackRead for &mut T {
    fn remaining(&self) -> usize {
        (**self).remaining()
    }

    fn read_bytes_exact(&mut self, buf: &mut [u8]) -> Result<(), PackError> {
        (**self).read_bytes_exact(buf)
    }

    fn read_fact_key(&mut self, keys: &mut FactStrings) -> Result<FactKey, PackError> {
        (**self).read_fact_key(keys)
    }
}

/// A reader whose reads and preallocation bounds cannot pass `length`.
pub(super) struct BoundedReader<'a, R> {
    reader: &'a mut R,
    remaining: usize,
}

impl<'a, R: BufRead> BoundedReader<'a, R> {
    pub(super) fn new(reader: &'a mut R, length: usize) -> Self {
        Self {
            reader,
            remaining: length,
        }
    }

    pub(super) fn drain_remaining(&mut self) -> Result<(), PackError> {
        let mut chunk = [0u8; 4096];
        while self.remaining > 0 {
            let n = self.remaining.min(chunk.len());
            self.read_bytes_exact(&mut chunk[..n])?;
        }
        Ok(())
    }
}

impl<R: BufRead> PackRead for BoundedReader<'_, R> {
    fn remaining(&self) -> usize {
        self.remaining
    }

    fn read_bytes_exact(&mut self, buf: &mut [u8]) -> Result<(), PackError> {
        if buf.len() > self.remaining {
            return Err(PackError::Truncated);
        }
        self.reader.read_exact(buf).map_err(map_stream_error)?;
        self.remaining -= buf.len();
        Ok(())
    }

    fn read_fact_key(&mut self, keys: &mut FactStrings) -> Result<FactKey, PackError> {
        let len = read_u32(self)? as usize;
        if len > self.remaining {
            return Err(PackError::Truncated);
        }
        let buffered_key = {
            let buffered = self.reader.fill_buf().map_err(map_stream_error)?;
            if buffered.len() >= len {
                let text = std::str::from_utf8(&buffered[..len])
                    .map_err(|_| PackError::BadLength("quest gate key is not UTF-8".into()))?;
                Some(FactKey(keys.intern(text)))
            } else {
                None
            }
        };
        if let Some(key) = buffered_key {
            self.reader.consume(len);
            self.remaining -= len;
            return Ok(key);
        }

        let mut bytes = vec![0; len];
        self.read_bytes_exact(&mut bytes)?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| PackError::BadLength("quest gate key is not UTF-8".into()))?;
        Ok(FactKey(keys.intern(text)))
    }
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
    read_magic(&mut r, MAGIC_GRID)?;
    decode_grid_body(r)
}

fn decode_grid_body<R: PackRead>(mut r: R) -> Result<StepGrid, PackError> {
    let version = read_u8(&mut r)?;
    if version != VERSION_GRID {
        return Err(PackError::BadVersion(version));
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
    if cells > r.remaining() {
        return Err(PackError::Truncated);
    }
    let mut walk = vec![0u8; cells];
    r.read_bytes_exact(&mut walk)?;
    let n_doors = read_u32(&mut r)? as usize;
    // Cap the preallocation at what the remaining bytes can hold; the reads
    // themselves still fail with Truncated past the real end.
    let remaining = r.remaining();
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

/// Dispatch a bounded stream by magic, preserving the legacy-grid fallback
/// and draining accepted trailing bytes for callers that hash while reading.
pub(super) fn decode_any_reader<R: BufRead, T>(
    reader: &mut R,
    length: usize,
    from_pack: impl FnOnce(WorldCollision, TransportGraph, Vec<BankStand>) -> T,
    from_grid: impl FnOnce(StepGrid) -> T,
) -> Result<T, PackError> {
    let mut r = BoundedReader::new(reader, length);
    let mut magic = [0u8; 4];
    r.read_bytes_exact(&mut magic)?;
    if &magic == MAGIC {
        let (collision, graph, banks) = decode_pack_body(&mut r)?;
        r.drain_remaining()?;
        Ok(from_pack(collision, graph, banks))
    } else if &magic == MAGIC_GRID {
        let grid = decode_grid_body(&mut r)?;
        r.drain_remaining()?;
        Ok(from_grid(grid))
    } else {
        Err(PackError::BadMagic)
    }
}

fn read_magic<R: PackRead>(r: &mut R, expected: &[u8; 4]) -> Result<(), PackError> {
    let mut magic = [0u8; 4];
    r.read_bytes_exact(&mut magic)?;
    if &magic == expected {
        Ok(())
    } else {
        Err(PackError::BadMagic)
    }
}

/// Read and decode the 274N grid at `path`.
pub fn load_grid(path: &Path) -> Result<StepGrid, PackError> {
    let bytes = std::fs::read(path).map_err(PackError::Io)?;
    decode_grid(&bytes)
}

/// Serialize the whole-world collision + transport graph + bank stand and
/// zone tables to the v16 pack byte format. The graph's `at` index is not
/// stored; [`decode`] rebuilds it from the edges and collision. Teleports
/// (kind-4 edges) are written after ordinary edges and always carry absent
/// approach geometry. The raw flags are not on the wire (see the flags
/// sidecar); the zone bucket index is rebuilt at decode.
///
/// Returns an error if an edge's quest gates name a family other than
/// [`TransportGraph::quest_family`], rather than silently rebinding the keys.
/// Invalid model data and unrepresentable sizes are also reported as errors.
pub fn encode(
    collision: &WorldCollision,
    graph: &TransportGraph,
    banks: &[BankStand],
) -> Result<Vec<u8>, PackError> {
    let capacity = encoded_size(collision, graph, banks)?;
    let mut out = Vec::new();
    out.try_reserve_exact(capacity).map_err(|error| {
        PackError::BadLength(format!(
            "cannot reserve {capacity} bytes for nav pack: {error}"
        ))
    })?;
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    write_quest_family(&mut out, graph.quest_family.as_ref());
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
    let edge_count = graph.edges.len() + graph.teleports.len();
    out.extend_from_slice(&(edge_count as u32).to_le_bytes());
    for (edge_index, e) in graph.edges.iter().chain(&graph.teleports).enumerate() {
        out.push(
            kind_to_u8(e.kind)?
                | if e.player_delta.is_some() {
                    PLAYER_RELATIVE
                } else {
                    0
                },
        );
        let to = e
            .landing_from(e.at)
            .ok_or_else(|| PackError::BadLength("transport landing overflows its anchor".into()))?;
        for v in [
            e.at.x, e.at.z, e.at.level, to.x, to.z, to.level, e.loc_id, e.option, e.ticks,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.push(dir_to_u8(e.dir));
        out.extend_from_slice(&e.open_loc_id.unwrap_or(-1).to_le_bytes());
        write_req_pairs(&mut out, &e.skill_req);
        write_req_pairs(&mut out, &e.item_req);
        write_req_pairs(&mut out, &e.consumed_req);
        write_req_pairs(&mut out, &e.item_returns);
        write_req_strings(&mut out, &e.quest_req);
        write_req_pairs(&mut out, &e.varp_req);
        write_req_ids(&mut out, &e.worn_req);
        write_req_ids(&mut out, &e.worn_all_req);
        out.push(u8::from(e.members_req));
        out.extend_from_slice(&e.wildy_cap.unwrap_or(-1).to_le_bytes());
        write_quest_gates(&mut out, e.quest_gates.as_ref());
        write_takeoff(&mut out, e.takeoff);
        write_approach(
            &mut out,
            if edge_index < graph.edges.len() {
                graph.approaches.get(edge_index).copied().flatten()
            } else {
                None
            },
        );
    }
    write_bank_stands(&mut out, banks)?;
    write_wilderness_rules(&mut out, &graph.wilderness)?;
    zones::write_table(&mut out, graph.zones.as_ref())?;
    if out.len() != capacity {
        return Err(PackError::BadLength(format!(
            "encoded pack length {} differs from checked size {capacity}",
            out.len()
        )));
    }
    Ok(out)
}

fn encoded_size(
    collision: &WorldCollision,
    graph: &TransportGraph,
    banks: &[BankStand],
) -> Result<usize, PackError> {
    if collision.width == 0
        || collision.height == 0
        || collision.width > MAX_GRID
        || collision.height > MAX_GRID
    {
        return Err(PackError::BadLength(format!(
            "grid {}x{} exceeds the {MAX_GRID} tile cap",
            collision.width, collision.height
        )));
    }
    let width = u32::try_from(collision.width)
        .map_err(|_| PackError::BadLength("grid width exceeds u32".into()))?;
    let height = u32::try_from(collision.height)
        .map_err(|_| PackError::BadLength("grid height exceeds u32".into()))?;
    let cells = collision
        .width
        .checked_mul(collision.height)
        .and_then(|cells| cells.checked_mul(4))
        .ok_or_else(|| PackError::BadLength("grid size overflows".into()))?;
    if collision.walk.len() != cells {
        return Err(PackError::BadLength(format!(
            "walk length {} differs from grid cell count {cells}",
            collision.walk.len()
        )));
    }
    let blocked_words = cells
        .checked_add(63)
        .ok_or_else(|| PackError::BadLength("blocked grid size overflows".into()))?
        / 64;
    if collision.blocked.len() != blocked_words {
        return Err(PackError::BadLength(format!(
            "blocked word count {} differs from expected {blocked_words}",
            collision.blocked.len()
        )));
    }

    let edge_count = graph
        .edges
        .len()
        .checked_add(graph.teleports.len())
        .ok_or_else(|| PackError::BadLength("transport edge count overflows".into()))?;
    checked_count(edge_count, "transport edge")?;
    let mut size = 4 + 1 + if graph.quest_family.is_some() { 35 } else { 1 };
    size = add_encoded_size(size, 12 + 8, "collision header")?;
    size = add_encoded_size(size, collision.walk.len(), "walk grid")?;
    size = add_encoded_size(
        size,
        collision
            .blocked
            .len()
            .checked_mul(8)
            .ok_or_else(|| PackError::BadLength("blocked grid size overflows".into()))?,
        "blocked grid",
    )?;
    size = add_encoded_size(size, 4, "transport edge count")?;

    for (edge_index, edge) in graph.edges.iter().chain(&graph.teleports).enumerate() {
        kind_to_u8(edge.kind)?;
        validate_resource_requirements(edge)?;
        if edge.player_delta.is_some()
            && !matches!(edge.kind, TransportKind::Ladder | TransportKind::Stairs)
        {
            return Err(PackError::BadLength(
                "player-relative landing is only valid for ladders and stairs".into(),
            ));
        }
        edge.landing_from(edge.at)
            .ok_or_else(|| PackError::BadLength("transport landing overflows its anchor".into()))?;
        if let Some(cap) = edge.wildy_cap {
            if cap < 0 {
                return Err(PackError::BadLength(
                    "wildy_cap must be non-negative or absent".into(),
                ));
            }
        }
        let approach = if edge_index < graph.edges.len() {
            graph.approaches.get(edge_index).copied().flatten()
        } else {
            None
        };
        validate_approach(edge.kind, approach)?;
        // The sizing helpers below add every vector and gate count prefix.
        let mut edge_size = 48usize;
        for (requirements, name) in [
            (&edge.skill_req, "skill requirements"),
            (&edge.item_req, "item requirements"),
            (&edge.consumed_req, "consumed requirements"),
            (&edge.item_returns, "item returns"),
            (&edge.varp_req, "varp requirements"),
        ] {
            size_pairs(&mut edge_size, requirements.len(), name)?;
        }
        size_strings(&mut edge_size, &edge.quest_req, "quest requirements")?;
        size_ids(&mut edge_size, edge.worn_req.len(), "worn requirements")?;
        size_ids(
            &mut edge_size,
            edge.worn_all_req.len(),
            "conjunctive worn requirements",
        )?;
        edge_size = add_encoded_size(edge_size, 13, "transport takeoff")?;
        size_quest_gates(&mut edge_size, edge.quest_gates.as_ref())?;
        if let Some(takeoff) = edge.takeoff {
            validate_takeoff_admission(collision, edge, takeoff, approach)?;
        }
        if approach.is_some() {
            edge_size = add_encoded_size(edge_size, 3, "approach geometry")?;
        }
        if let Some(gates) = &edge.quest_gates {
            if graph.quest_family.as_ref() != Some(gates.family()) {
                return Err(PackError::BadLength(format!(
                    "{:?} loc {} carries quest gates of another quest family than the pack's",
                    edge.kind, edge.loc_id
                )));
            }
        }
        size = add_encoded_size(size, edge_size, "transport edge")?;
    }

    checked_count(banks.len(), "bank stand")?;
    size = add_encoded_size(size, 4, "bank stand count")?;
    for bank in banks {
        size_string(&mut size, &bank.name, "bank stand name")?;
        size = add_encoded_size(size, 12, "bank stand tile")?;
        match &bank.access {
            BankAccess::Booth { .. } => {
                size = add_encoded_size(size, 5, "booth bank access")?;
            }
            BankAccess::Npc { name, choose, .. } => {
                size = add_encoded_size(size, 1, "NPC bank access tag")?;
                size_string(&mut size, name, "bank teller name")?;
                size = add_encoded_size(size, 4, "NPC bank operation")?;
                size = add_encoded_size(size, 1, "bank dialog choice tag")?;
                if let Some(choice) = choose {
                    size_string(&mut size, choice, "bank dialog choice")?;
                }
            }
        }
    }

    checked_count(graph.wilderness.zones.len(), "wilderness zone")?;
    let wilderness_size = graph
        .wilderness
        .zones
        .len()
        .checked_mul(28)
        .and_then(|zones| zones.checked_add(12))
        .ok_or_else(|| PackError::BadLength("wilderness table size overflows".into()))?;
    size = add_encoded_size(size, wilderness_size, "wilderness table")?;

    if let Some(table) = graph.zones.as_ref() {
        if table.bounds() != (collision.origin, width, height) {
            return Err(PackError::BadLength(
                "zone table bounds must match packed collision".into(),
            ));
        }
    }
    size = add_encoded_size(size, zones::wire_size(graph.zones.as_ref())?, "zone table")?;
    Ok(size)
}

fn checked_count(count: usize, what: &str) -> Result<u32, PackError> {
    u32::try_from(count).map_err(|_| PackError::BadLength(format!("{what} count exceeds u32")))
}

fn add_encoded_size(total: usize, extra: usize, what: &str) -> Result<usize, PackError> {
    total
        .checked_add(extra)
        .ok_or_else(|| PackError::BadLength(format!("{what} size overflows usize")))
}

fn size_string(total: &mut usize, value: &str, what: &str) -> Result<(), PackError> {
    u32::try_from(value.len())
        .map_err(|_| PackError::BadLength(format!("{what} length exceeds u32")))?;
    *total = add_encoded_size(
        *total,
        value
            .len()
            .checked_add(4)
            .ok_or_else(|| PackError::BadLength(format!("{what} size overflows usize")))?,
        what,
    )?;
    Ok(())
}

fn size_pairs(total: &mut usize, count: usize, what: &str) -> Result<(), PackError> {
    checked_count(count, what)?;
    let bytes = count
        .checked_mul(8)
        .and_then(|bytes| bytes.checked_add(4))
        .ok_or_else(|| PackError::BadLength(format!("{what} size overflows usize")))?;
    *total = add_encoded_size(*total, bytes, what)?;
    Ok(())
}

fn size_ids(total: &mut usize, count: usize, what: &str) -> Result<(), PackError> {
    checked_count(count, what)?;
    let bytes = count
        .checked_mul(4)
        .and_then(|bytes| bytes.checked_add(4))
        .ok_or_else(|| PackError::BadLength(format!("{what} size overflows usize")))?;
    *total = add_encoded_size(*total, bytes, what)?;
    Ok(())
}

fn size_strings(total: &mut usize, strings: &[String], what: &str) -> Result<(), PackError> {
    checked_count(strings.len(), what)?;
    *total = add_encoded_size(*total, 4, what)?;
    for string in strings {
        size_string(total, string, what)?;
    }
    Ok(())
}

fn size_key(total: &mut usize, key: &FactKey) -> Result<(), PackError> {
    size_string(total, &key.0, "quest gate key")
}

fn size_quest_gates(total: &mut usize, gates: Option<&QuestGates>) -> Result<(), PackError> {
    let gates = gates.map_or(&[][..], QuestGates::gates);
    checked_count(gates.len(), "quest gate")?;
    *total = add_encoded_size(*total, 4, "quest gate count")?;
    for gate in gates {
        *total = add_encoded_size(*total, 1, "quest gate tag")?;
        match gate {
            QuestGate::Complete(quest) => size_key(total, quest)?,
            QuestGate::Window(window) => {
                size_key(total, &window.quest)?;
                size_key(total, &window.signal)?;
                for bound in [window.values.min, window.values.max] {
                    *total = add_encoded_size(
                        *total,
                        if bound.is_some() { 5 } else { 1 },
                        "quest gate bound",
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn validate_approach(kind: TransportKind, approach: Option<LocApproach>) -> Result<(), PackError> {
    if let Some(approach) = approach {
        if approach.width == 0 || approach.length == 0 {
            return Err(PackError::BadLength(
                "approach geometry dimensions must be positive".into(),
            ));
        }
        if approach.blocked_sides & !0x0f != 0 {
            return Err(PackError::BadLength(format!(
                "approach blocked-side mask {:#04x} exceeds 0x0f",
                approach.blocked_sides
            )));
        }
        if !matches!(
            kind,
            TransportKind::Ladder
                | TransportKind::Stairs
                | TransportKind::AgilityShortcut
                | TransportKind::SpiritTree
        ) {
            return Err(PackError::BadLength(format!(
                "approach geometry is not valid for {kind:?}"
            )));
        }
    }
    Ok(())
}

/// `TransportKind` as a wire byte.
fn kind_to_u8(k: TransportKind) -> Result<u8, PackError> {
    match k {
        TransportKind::Door => Ok(0),
        TransportKind::Ladder => Ok(1),
        TransportKind::Stairs => Ok(2),
        TransportKind::Boat => Ok(3),
        TransportKind::Teleport => Ok(4),
        TransportKind::AgilityShortcut => Ok(5),
        TransportKind::Glider => Ok(6),
        TransportKind::SpiritTree => Ok(7),
        TransportKind::Npc => Ok(8),
        TransportKind::EssenceExit => Err(PackError::BadLength(
            "the essence return is synthesized at runtime and cannot be packed".into(),
        )),
    }
}

/// Deserialize the whole-world pack, validating magic, version, and lengths.
/// The `at` and zone bucket indices are rebuilt from their packed tables.
/// Version 16 is the only accepted wire; older streams are rejected rather
/// than compat-loaded. Quest-stage gates bind to the header's quest family;
/// malformed resource requirements, gates, or approach geometry are rejected.
pub fn decode(bytes: &[u8]) -> Result<(WorldCollision, TransportGraph, Vec<BankStand>), PackError> {
    let mut r = Cursor::new(bytes);
    read_magic(&mut r, MAGIC)?;
    decode_pack_body(r)
}

fn decode_pack_body<R: PackRead>(
    mut r: R,
) -> Result<(WorldCollision, TransportGraph, Vec<BankStand>), PackError> {
    let version = read_u8(&mut r)?;
    if version != VERSION {
        return Err(PackError::BadVersion(version));
    }
    let quest_family = read_quest_family(&mut r)?;
    // Gate keys share one allocation per distinct string; the table itself
    // is dropped with this decode.
    let mut keys = FactStrings::default();
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
    if cells > r.remaining() {
        return Err(PackError::Truncated);
    }
    let mut walk = vec![0u8; cells];
    r.read_bytes_exact(&mut walk)?;
    let words = cells.div_ceil(64);
    let remaining = r.remaining();
    let mut blocked = Vec::with_capacity(words.min(remaining / 8));
    for _ in 0..words {
        blocked.push(read_u64(&mut r)?);
    }
    let n_edges = read_u32(&mut r)? as usize;
    // Cap the preallocation at what the remaining bytes can hold; the reads
    // themselves still fail with Truncated past the real end.
    let remaining = r.remaining();
    let mut graph = TransportGraph {
        // A v16 edge is at least 97 bytes, including its takeoff field and trailer.
        edges: Vec::with_capacity(n_edges.min(remaining / 97)),
        approaches: Vec::with_capacity(n_edges.min(remaining / 97)),
        quest_family,
        ..Default::default()
    };
    for _ in 0..n_edges {
        let packed_kind = read_u8(&mut r)?;
        let kind = kind_from_u8(packed_kind & !PLAYER_RELATIVE)?;
        let player_relative = packed_kind & PLAYER_RELATIVE != 0;
        if player_relative && !matches!(kind, TransportKind::Ladder | TransportKind::Stairs) {
            return Err(PackError::BadLength(
                "player-relative landing is only valid for ladders and stairs".into(),
            ));
        }
        let mut edge = TransportEdge {
            kind,
            player_delta: None,
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
            consumed_req: read_req_pairs(&mut r)?,
            item_returns: read_req_pairs(&mut r)?,
            quest_req: read_req_strings(&mut r)?,
            varp_req: read_req_pairs(&mut r)?,
            worn_req: read_req_ids(&mut r)?,
            worn_all_req: read_req_ids(&mut r)?,
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
            quest_gates: read_quest_gates(&mut r, quest_family.as_ref(), &mut keys)?,
            takeoff: None,
        };
        edge.takeoff = read_takeoff(&mut r, edge.at)?;
        validate_resource_requirements(&edge)?;
        if player_relative {
            let delta = |to: i32, at: i32| {
                to.checked_sub(at).ok_or_else(|| {
                    PackError::BadLength("player-relative displacement overflows i32".into())
                })
            };
            edge.player_delta = Some(WorldTile {
                x: delta(edge.to.x, edge.at.x)?,
                z: delta(edge.to.z, edge.at.z)?,
                level: delta(edge.to.level, edge.at.level)?,
            });
        }
        let approach = read_approach(&mut r, kind)?;
        if kind == TransportKind::Teleport {
            graph.teleports.push(edge);
        } else {
            graph.edges.push(edge);
            graph.approaches.push(approach);
        }
    }
    let banks = read_bank_stands(&mut r)?;
    graph.wilderness = read_wilderness_rules(&mut r)?;
    let zone_table = zones::read_table(
        &mut r,
        origin,
        width as u32,
        height as u32,
        &graph.wilderness,
        &mut keys,
    )?;
    graph.zones = zone_table;
    let collision = WorldCollision {
        origin,
        width,
        height,
        // The packed walk surface is the resident form; the raw flags
        // live only in the sidecar (loaded on demand for debug paints).
        walk,
        blocked,
        flags: None,
    };
    for (index, edge) in graph.edges.iter().enumerate() {
        if let Some(takeoff) = edge.takeoff {
            validate_takeoff_admission(
                &collision,
                edge,
                takeoff,
                graph.approaches.get(index).copied().flatten(),
            )?;
        }
    }
    for edge in &graph.teleports {
        if let Some(takeoff) = edge.takeoff {
            validate_takeoff_admission(&collision, edge, takeoff, None)?;
        }
    }
    graph.rebuild_index(&collision);
    Ok((collision, graph, banks))
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
fn read_req_pairs<R: PackRead>(r: &mut R) -> Result<Vec<(i32, i32)>, PackError> {
    let n = read_u32(r)? as usize;
    let remaining = r.remaining();
    let mut out = Vec::with_capacity(n.min(remaining / 8));
    for _ in 0..n {
        out.push((read_i32(r)?, read_i32(r)?));
    }
    Ok(out)
}

/// Read a count-prefixed length-prefixed UTF-8 string vector.
fn read_req_strings<R: PackRead>(r: &mut R) -> Result<Vec<String>, PackError> {
    let n = read_u32(r)? as usize;
    let remaining = r.remaining();
    let mut out = Vec::with_capacity(n.min(remaining / 4));
    for _ in 0..n {
        let len = read_u32(r)? as usize;
        if len > r.remaining() {
            return Err(PackError::Truncated);
        }
        let mut buf = vec![0u8; len];
        r.read_bytes_exact(&mut buf)?;
        let s = String::from_utf8(buf)
            .map_err(|_| PackError::BadLength("quest req is not UTF-8".into()))?;
        out.push(s);
    }
    Ok(out)
}

/// A worn-item requirement vector as count-prefixed i32le ids.
fn write_req_ids(out: &mut Vec<u8>, reqs: &[i32]) {
    out.extend_from_slice(&(reqs.len() as u32).to_le_bytes());
    for id in reqs {
        out.extend_from_slice(&id.to_le_bytes());
    }
}

/// Read a count-prefixed i32le id vector.
fn read_req_ids<R: PackRead>(r: &mut R) -> Result<Vec<i32>, PackError> {
    let n = read_u32(r)? as usize;
    let remaining = r.remaining();
    let mut out = Vec::with_capacity(n.min(remaining / 4));
    for _ in 0..n {
        out.push(read_i32(r)?);
    }
    Ok(out)
}

/// Validate item requirement counts and the replacement-after-consumption invariant.
fn validate_resource_requirements(edge: &TransportEdge) -> Result<(), PackError> {
    for (field, requirements) in [
        ("item_req", &edge.item_req),
        ("consumed_req", &edge.consumed_req),
        ("item_returns", &edge.item_returns),
    ] {
        if requirements.iter().any(|(_, count)| *count <= 0) {
            return Err(PackError::BadLength(format!(
                "{field} counts must be positive"
            )));
        }
    }
    if !edge.item_returns.is_empty() && edge.consumed_req.is_empty() {
        return Err(PackError::BadLength(
            "item_returns require a consumed_req".into(),
        ));
    }
    Ok(())
}

/// The v11 header's quest-family binding: `0`, or `1` + digest + schema.
fn write_quest_family(out: &mut Vec<u8>, family: Option<&QuestFamilyId>) {
    match family {
        None => out.push(0),
        Some(family) => {
            out.push(1);
            out.extend_from_slice(family.quest_facts_sha256());
            out.extend_from_slice(&family.quest_extractor_schema().get().to_le_bytes());
        }
    }
}

/// Read the header's quest-family binding; no [`QuestFamilyId`] has
/// extractor schema 0, so it is malformed rather than "no schema".
fn read_quest_family<R: PackRead>(r: &mut R) -> Result<Option<QuestFamilyId>, PackError> {
    match read_u8(r)? {
        0 => Ok(None),
        1 => {
            let mut quest_facts_sha256 = [0u8; 32];
            r.read_bytes_exact(&mut quest_facts_sha256)?;
            let quest_extractor_schema = NonZeroU16::new(read_u16(r)?)
                .ok_or_else(|| PackError::BadLength("quest family extractor schema is 0".into()))?;
            Ok(Some(QuestFamilyId::new(
                quest_facts_sha256,
                quest_extractor_schema,
            )))
        }
        other => Err(PackError::BadLength(format!(
            "quest family binding tag {other} is not 0 or 1"
        ))),
    }
}

/// An edge's quest-stage gates, count-prefixed (`0` = ungated).
fn write_quest_gates(out: &mut Vec<u8>, gates: Option<&QuestGates>) {
    let gates = gates.map_or(&[][..], QuestGates::gates);
    out.extend_from_slice(&(gates.len() as u32).to_le_bytes());
    for gate in gates {
        match gate {
            QuestGate::Complete(quest) => {
                out.push(0);
                write_key(out, quest);
            }
            QuestGate::Window(window) => {
                out.push(1);
                write_key(out, &window.quest);
                write_key(out, &window.signal);
                write_bound(out, window.values.min);
                write_bound(out, window.values.max);
            }
        }
    }
}

/// Read an edge's quest-stage gates, bound to the header's `family`. Gates
/// on a pack that binds no family, or that fail [`QuestGates::new`]'s
/// validation (an empty key, an empty or unbounded window), are malformed.
fn read_quest_gates<R: PackRead>(
    r: &mut R,
    family: Option<&QuestFamilyId>,
    keys: &mut FactStrings,
) -> Result<Option<QuestGates>, PackError> {
    let n = read_u32(r)? as usize;
    if n == 0 {
        return Ok(None);
    }
    let Some(family) = family else {
        return Err(PackError::BadLength(
            "quest gates without a quest-family binding".into(),
        ));
    };
    let remaining = r.remaining();
    let mut gates = Vec::with_capacity(n.min(remaining / 6));
    for _ in 0..n {
        gates.push(match read_u8(r)? {
            0 => QuestGate::Complete(read_key(r, keys)?),
            1 => QuestGate::Window(StageWindow {
                quest: read_key(r, keys)?,
                signal: read_key(r, keys)?,
                values: InclusiveRange {
                    min: read_bound(r)?,
                    max: read_bound(r)?,
                },
            }),
            other => {
                return Err(PackError::BadLength(format!(
                    "quest gate tag {other} is not 0 or 1"
                )));
            }
        });
    }
    QuestGates::new(*family, gates)
        .map(Some)
        .map_err(|error| PackError::BadLength(error.to_string()))
}

/// Write a fixed-size v16 exact-takeoff field.
fn write_takeoff(out: &mut Vec<u8>, takeoff: Option<WorldTile>) {
    match takeoff {
        None => {
            out.push(0);
            out.extend_from_slice(&[0; 12]);
        }
        Some(tile) => {
            out.push(1);
            for value in [tile.x, tile.z, tile.level] {
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
    }
}

fn validate_takeoff(at: WorldTile, takeoff: WorldTile) -> Result<(), PackError> {
    if !(0..4).contains(&takeoff.level) || takeoff.level != at.level {
        return Err(PackError::BadLength(format!(
            "transport takeoff plane {} does not match valid anchor plane {}",
            takeoff.level, at.level
        )));
    }
    Ok(())
}
fn validate_takeoff_admission(
    collision: &WorldCollision,
    edge: &TransportEdge,
    takeoff: WorldTile,
    approach: Option<LocApproach>,
) -> Result<(), PackError> {
    validate_takeoff(edge.at, takeoff)?;
    if !collision.standable(takeoff) {
        return Err(PackError::BadLength(format!(
            "transport takeoff {takeoff:?} is not standable"
        )));
    }
    if edge.kind == TransportKind::Teleport {
        return Ok(());
    }
    let admissible = approach.map_or_else(
        || {
            (takeoff.x - edge.at.x)
                .abs()
                .max((takeoff.z - edge.at.z).abs())
                <= 1
        },
        |shape| {
            shape.can_operate(
                edge.at,
                takeoff,
                collision.walkable_word(takeoff.x, takeoff.z, takeoff.level) as i32,
            )
        },
    );
    if !admissible {
        return Err(PackError::BadLength(format!(
            "transport takeoff {takeoff:?} is not an admissible stand for {:?} at {:?}",
            edge.kind, edge.at
        )));
    }
    Ok(())
}

fn read_takeoff<R: PackRead>(r: &mut R, at: WorldTile) -> Result<Option<WorldTile>, PackError> {
    let tag = read_u8(r)?;
    let tile = WorldTile {
        x: read_i32(r)?,
        z: read_i32(r)?,
        level: read_i32(r)?,
    };
    match tag {
        0 if tile.x == 0 && tile.z == 0 && tile.level == 0 => Ok(None),
        0 => Err(PackError::BadLength(
            "absent transport takeoff has nonzero payload".into(),
        )),
        1 => {
            validate_takeoff(at, tile)?;
            Ok(Some(tile))
        }
        other => Err(PackError::BadLength(format!(
            "transport takeoff tag {other} is not 0 or 1"
        ))),
    }
}

/// Write the v13 per-edge footprint-approach geometry.
fn write_approach(out: &mut Vec<u8>, approach: Option<LocApproach>) {
    match approach {
        None => out.push(0),
        Some(approach) => {
            out.push(1);
            out.extend_from_slice(&[approach.width, approach.length, approach.blocked_sides]);
        }
    }
}

/// Read and validate an edge's v13 footprint-approach geometry.
fn read_approach<R: PackRead>(
    r: &mut R,
    kind: TransportKind,
) -> Result<Option<LocApproach>, PackError> {
    match read_u8(r)? {
        0 => Ok(None),
        1 => {
            let approach = LocApproach {
                width: read_u8(r)?,
                length: read_u8(r)?,
                blocked_sides: read_u8(r)?,
            };
            if approach.width == 0 || approach.length == 0 {
                return Err(PackError::BadLength(
                    "approach geometry dimensions must be positive".into(),
                ));
            }
            if approach.blocked_sides & !0x0f != 0 {
                return Err(PackError::BadLength(format!(
                    "approach blocked-side mask {:#04x} exceeds 0x0f",
                    approach.blocked_sides
                )));
            }
            if !matches!(
                kind,
                TransportKind::Ladder
                    | TransportKind::Stairs
                    | TransportKind::AgilityShortcut
                    | TransportKind::SpiritTree
            ) {
                return Err(PackError::BadLength(format!(
                    "approach geometry is not valid for {kind:?}"
                )));
            }
            Ok(Some(approach))
        }
        tag => Err(PackError::BadLength(format!(
            "approach geometry tag {tag} is not 0 or 1"
        ))),
    }
}

/// A fact key as a length-prefixed UTF-8 string.
fn write_key(out: &mut Vec<u8>, key: &FactKey) {
    out.extend_from_slice(&(key.0.len() as u32).to_le_bytes());
    out.extend_from_slice(key.0.as_bytes());
}

/// Read a length-prefixed UTF-8 fact key through the decode's interner.
fn read_key<R: PackRead>(r: &mut R, keys: &mut FactStrings) -> Result<FactKey, PackError> {
    r.read_fact_key(keys)
}

/// One stage-window bound: `0` unbounded, `1` + i32le.
fn write_bound(out: &mut Vec<u8>, bound: Option<i32>) {
    match bound {
        None => out.push(0),
        Some(value) => {
            out.push(1);
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
}

fn read_bound<R: PackRead>(r: &mut R) -> Result<Option<i32>, PackError> {
    match read_u8(r)? {
        0 => Ok(None),
        1 => Ok(Some(read_i32(r)?)),
        other => Err(PackError::BadLength(format!(
            "stage window bound flag {other} is not 0 or 1"
        ))),
    }
}

fn write_wilderness_rules(
    out: &mut Vec<u8>,
    rules: &crate::transport::WildernessRules,
) -> Result<(), PackError> {
    out.extend_from_slice(
        &u32::try_from(rules.zones.len())
            .map_err(|_| PackError::BadLength("wilderness zone count exceeds u32".into()))?
            .to_le_bytes(),
    );
    for z in &rules.zones {
        for v in [z.x1, z.z1, z.x2, z.z2, z.level1, z.level2, z.origin_z] {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out.extend_from_slice(&rules.divisor.to_le_bytes());
    out.extend_from_slice(&rules.offset.to_le_bytes());
    Ok(())
}

fn read_wilderness_rules<R: PackRead>(
    r: &mut R,
) -> Result<crate::transport::WildernessRules, PackError> {
    let n = read_u32(r)? as usize;
    let remaining = r.remaining();
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

fn read_i32<R: PackRead>(r: &mut R) -> Result<i32, PackError> {
    let mut b = [0u8; 4];
    r.read_bytes_exact(&mut b)?;
    Ok(i32::from_le_bytes(b))
}

fn read_u8<R: PackRead>(r: &mut R) -> Result<u8, PackError> {
    let mut b = [0u8; 1];
    r.read_bytes_exact(&mut b)?;
    Ok(b[0])
}

fn read_u16<R: PackRead>(r: &mut R) -> Result<u16, PackError> {
    let mut b = [0u8; 2];
    r.read_bytes_exact(&mut b)?;
    Ok(u16::from_le_bytes(b))
}

fn read_u32<R: PackRead>(r: &mut R) -> Result<u32, PackError> {
    let mut b = [0u8; 4];
    r.read_bytes_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

fn read_u64<R: PackRead>(r: &mut R) -> Result<u64, PackError> {
    let mut b = [0u8; 8];
    r.read_bytes_exact(&mut b)?;
    Ok(u64::from_le_bytes(b))
}

#[cfg(test)]
#[path = "pack_tests.rs"]
mod tests;
