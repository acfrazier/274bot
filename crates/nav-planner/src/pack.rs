//! Runtime decoders for the whole-world 274V pack and legacy 274N grid.
//! The `nav-bake` crate owns pack/grid encoding and content construction.
//!
//! Grid format (274N): magic `b"274N"`, version `u8` 1, origin
//! `(x, z, level)` i32le, width/height u32le, one walk byte per tile
//! (row-major z then x, 1 = walkable, same indexing as [`StepGrid`]),
//! door count u32le, then per door `(loc_x, loc_z, loc_level, loc_id,
//! from_x, from_z, from_level, to_x, to_z, to_level)` all i32le. Door loc
//! ids come from the Server `content/scripts/doors/configs/*.loc` blocks.
//! The parser lives in `nav-bake::pack`. Blocking loc footprints come from
//! `[loc_N]` `blockwalk` (default yes).
//!
//! Pack format (274V): magic `b"274V"`, version `u8` 17, the quest-family
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
//! Every decoded v17 stream has `Some(ZoneTable)`, even when every row count
//! is zero; legacy grids and synthetic in-memory graphs use `zones: None`.
//! Zone counts/indices are bounded to the packed namespaces; malformed rows
//! return [`PackError::BadLength`]. A v15 or older whole-world stream is
//! [`PackError::BadVersion`], never compat-loaded. Takeoff levels must match
//! the loc anchor's valid game plane, and exact tiles must be standable in
//! the decoded collision.

use std::fmt;
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
mod mapsquare;
mod sidecars;
mod zones;
use banks::read_bank_stands;
pub use banks::{BankAccess, BankStand};
pub use mapsquare::walkable_dots;
pub use sidecars::{
    decode_canlight_sidecar, decode_flags_sidecar, decode_flags_sidecar_arc, decode_reach_sidecar,
    read_canlight_sidecar, read_flags_sidecar, read_reach_sidecar, read_reach_sidecar_header,
    sha256_from_hex, sha256_hex, CanlightSidecar, CanlightSidecarLoad, FlagsSidecarLoad,
    ReachSidecar, ReachSidecarHeader, ReachSidecarLoad, FLAGS_HEADER_LEN, MAGIC_CANLIGHT,
    MAGIC_FLAGS, MAGIC_REACH, VERSION_CANLIGHT, VERSION_FLAGS, VERSION_REACH,
};
pub use zones::MAX_PACKED_INDEX;

/// Grid (274N) format version: boolean walk bytes + doors.
pub const VERSION_GRID: u8 = 1;

/// Grid (274N) file magic.
pub const MAGIC_GRID: &[u8; 4] = b"274N";

/// Current pack format version (collision + transport graph). v3 adds the
/// per-edge `dir`/`open_loc_id` fields, v4 stores the four collision planes,
/// v5 adds the per-edge worn-item id list `worn_req`, and v6 replaces the
/// four u32 flag planes with compact packed walk words. v7 splits those
/// words into the `u8` face byte plus packed `SQ_BLOCKED` bit-plane. v8
/// appends the bank stand table; v9 adds `members_req`; v10 adds wilderness
/// teleport caps and the Wilderness-level formula. v11 binds the selected
/// quest family and appends typed quest-stage gates. v12 appends the zone
/// table, v13 approach geometry, v14 player-relative landings, v15 consumed
/// resources and returns, v16 `worn_all_req` plus optional exact takeoff,
/// and v17 observed held/equipment exclusion requirements.
pub const VERSION: u8 = 17;

/// Current pack file magic.
pub const MAGIC: &[u8; 4] = b"274V";

/// Pack format identity as it appears in bundled navigation identities.
/// A format improvement changes this identity and invalidates staged builds.
pub const FORMAT_ID: &str = "274V17";

/// Bytes per door entry.
pub const DOOR_BYTES: usize = 40;

/// High bit of a ladder/stairs kind byte: `to - at` is a player displacement.
pub const PLAYER_RELATIVE: u8 = 0x80;

/// Largest grid side a pack may decode (16384×16384 tiles ≈ 256 MB of walk
/// bytes; the whole-world bbox is 1792×9088, comfortably under).
pub const MAX_GRID: usize = 16384;

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
            forbidden_held: read_req_ids(&mut r)?,
            forbidden_worn: read_req_ids(&mut r)?,
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
pub fn validate_resource_requirements(edge: &TransportEdge) -> Result<(), PackError> {
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

fn validate_takeoff(at: WorldTile, takeoff: WorldTile) -> Result<(), PackError> {
    if !(0..4).contains(&takeoff.level) || takeoff.level != at.level {
        return Err(PackError::BadLength(format!(
            "transport takeoff plane {} does not match valid anchor plane {}",
            takeoff.level, at.level
        )));
    }
    Ok(())
}
/// Validate the exact takeoff against pack collision and edge geometry.
pub fn validate_takeoff_admission(
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

/// Read a length-prefixed UTF-8 fact key through the decode's interner.
fn read_key<R: PackRead>(r: &mut R, keys: &mut FactStrings) -> Result<FactKey, PackError> {
    r.read_fact_key(keys)
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
