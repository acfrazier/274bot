//! Nav pack: encode/decode of [`StepGrid`] (v1) and of the whole-world
//! [`WorldCollision`] + [`TransportGraph`] (v2) to the `.navpack` binary
//! format, plus the jm2 mapsquare bake used by the `nav-pack` binary.
//!
//! V1 format: magic `b"274N"`, version `u8` 1, origin `(x, z, level)` i32le,
//! width/height u32le, one walk byte per tile (row-major z then x, 1 =
//! walkable, same indexing as [`StepGrid`]), door count u32le, then per door
//! `(loc_x, loc_z, loc_level, loc_id, from_x, from_z, from_level, to_x, to_z,
//! to_level)` all i32le. Door loc ids come from the Server
//! `content/scripts/doors/configs/*.loc` blocks (see [`parse_door_config`]).
//! Blocking loc footprints come from `[loc_N]` `blockwalk` (default yes).
//!
//! V2 format: magic `b"274V"`, version `u8` 2, collision origin
//! `(x, z, level)` i32le, width/height u32le, the [`WorldCollision`] `flags`
//! u32le per tile (row-major z then x), then the transport edge count u32le
//! and per edge `(kind u8, from x/z/level, to x/z/level, loc_id, option,
//! ticks)` i32le plus the four requirement vectors (count u32le, then
//! `(id, value)` i32le pairs; quest names as length-prefixed UTF-8). The
//! any-tile teleport layer (`TransportGraph::teleports`) round-trips inside
//! the same edges array as kind-4 edges; [`decode_v2`] splits them back out
//! and never indexes them into `from`. The
//! v1 decode stays for old `.navpack` files; `nav-pack` now writes v2.

use std::collections::HashSet;
use std::fmt;
use std::io::{self, Cursor, Read};
use std::path::Path;

use api::snapshot::WorldTile;

use crate::collision::WorldCollision;
use crate::grid::{DoorEdge, StepGrid};
use crate::tile::Tile;
use crate::transport::{TransportEdge, TransportGraph, TransportKind};

/// Current pack format version (v1: boolean walk bytes + doors).
const VERSION: u8 = 1;
/// File magic.
const MAGIC: &[u8; 4] = b"274N";
/// V2 pack format version (collision flags + transport graph).
const VERSION_V2: u8 = 2;
/// V2 file magic.
const MAGIC_V2: &[u8; 4] = b"274V";
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
    /// File does not start with the `b"274N"` magic.
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

/// Serialize `g` to the nav pack byte format.
pub fn encode(g: &StepGrid) -> Vec<u8> {
    let mut out =
        Vec::with_capacity(4 + 1 + 12 + 8 + g.walk.len() + 4 + g.doors.len() * DOOR_BYTES);
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
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

/// Deserialize a nav pack, validating magic, version, and lengths.
pub fn decode(bytes: &[u8]) -> Result<StepGrid, PackError> {
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

/// Read and decode the pack at `path`.
pub fn load_pack(path: &Path) -> Result<StepGrid, PackError> {
    let bytes = std::fs::read(path).map_err(PackError::Io)?;
    decode(&bytes)
}

/// Serialize the whole-world collision + transport graph to the v2 pack
/// byte format. The graph's `from` index is not stored; [`decode_v2`]
/// rebuilds it from the edges. Teleports (kind-4 edges) are written after
/// the ordinary edges in the same array — the wire layout is unchanged, so
/// old v2 packs decode identically (they just carry no kind-4 edges).
pub fn encode_v2(collision: &WorldCollision, graph: &TransportGraph) -> Vec<u8> {
    let edge_count = graph.edges.len() + graph.teleports.len();
    let mut out = Vec::with_capacity(
        4 + 1 + 12 + 8 + collision.flags.len() * 4 + 4 + edge_count * 88,
    );
    out.extend_from_slice(MAGIC_V2);
    out.push(VERSION_V2);
    for v in [collision.origin.x, collision.origin.z, collision.origin.level] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&(collision.width as u32).to_le_bytes());
    out.extend_from_slice(&(collision.height as u32).to_le_bytes());
    for f in &collision.flags {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out.extend_from_slice(&(edge_count as u32).to_le_bytes());
    for e in graph.edges.iter().chain(&graph.teleports) {
        out.push(kind_to_u8(e.kind));
        for v in [
            e.from.x,
            e.from.z,
            e.from.level,
            e.to.x,
            e.to.z,
            e.to.level,
            e.loc_id,
            e.option,
            e.ticks,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        write_req_pairs(&mut out, &e.skill_req);
        write_req_pairs(&mut out, &e.item_req);
        write_req_strings(&mut out, &e.quest_req);
        write_req_pairs(&mut out, &e.varp_req);
    }
    out
}

/// Deserialize a v2 pack, validating magic, version, and lengths. The
/// `from` index is rebuilt from the decoded edges; kind-4 (teleport) edges
/// split back into [`TransportGraph::teleports`] and are excluded from it.
pub fn decode_v2(bytes: &[u8]) -> Result<(WorldCollision, TransportGraph), PackError> {
    let mut r = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    r.read_exact(&mut magic).map_err(|_| PackError::Truncated)?;
    if &magic != MAGIC_V2 {
        return Err(PackError::BadMagic);
    }
    let mut version = [0u8; 1];
    r.read_exact(&mut version)
        .map_err(|_| PackError::Truncated)?;
    if version[0] != VERSION_V2 {
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
    let cells = width
        .checked_mul(height)
        .ok_or_else(|| PackError::BadLength("grid size overflows".into()))?;
    let mut flags = vec![0u32; cells];
    for f in &mut flags {
        *f = read_u32(&mut r)?;
    }
    let n_edges = read_u32(&mut r)? as usize;
    // Cap the preallocation at what the remaining bytes can hold; the reads
    // themselves still fail with Truncated past the real end.
    let remaining = bytes.len().saturating_sub(r.position() as usize);
    let mut graph = TransportGraph::default();
    graph.edges = Vec::with_capacity(n_edges.min(remaining / 36));
    for _ in 0..n_edges {
        let mut kind = [0u8; 1];
        r.read_exact(&mut kind).map_err(|_| PackError::Truncated)?;
        let edge = TransportEdge {
            kind: kind_from_u8(kind[0])?,
            from: WorldTile {
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
            skill_req: read_req_pairs(&mut r)?,
            item_req: read_req_pairs(&mut r)?,
            quest_req: read_req_strings(&mut r)?,
            varp_req: read_req_pairs(&mut r)?,
        };
        if edge.kind == TransportKind::Teleport {
            graph.teleports.push(edge);
        } else {
            graph.edges.push(edge);
        }
    }
    for (i, e) in graph.edges.iter().enumerate() {
        graph.from.entry(e.from).or_default().push(i);
    }
    Ok((
        WorldCollision {
            origin,
            width,
            height,
            // The derived walkable word is a pure function of the raw
            // flags, so it is recomputed rather than stored on the wire.
            walkable: crate::collision::derive_walkable(&flags),
            flags,
        },
        graph,
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
        _ => Err(PackError::BadLength(format!(
            "unknown transport kind {b}"
        ))),
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
/// `category=door_closed`. Non-numeric blocks (e.g. `[membergatel]`) are
/// ignored, as are the `op1=Close`/`*_opened` counterpart states.
pub fn parse_door_config(text: &str) -> HashSet<i32> {
    let mut ids = HashSet::new();
    let mut cur: Option<i32> = None;
    let mut openable = false;
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(n) = loc_header(line) {
            if let Some(id) = cur {
                if openable {
                    ids.insert(id);
                }
            }
            cur = Some(n);
            openable = false;
        } else if cur.is_some() && (line == "op1=Open" || line == "category=door_closed") {
            openable = true;
        }
    }
    if let Some(id) = cur {
        if openable {
            ids.insert(id);
        }
    }
    ids
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

/// Parse one mapsquare jm2 file (level 0 only). A MAP flag with bit 0 set
/// (`fN`, BLOCK_MAP_SQUARE) is blocked; tiles without a MAP line are not
/// walkable. A LOC whose loc id is in `door_ids` (openable wall doors from
/// the Server door configs) with shape 0 becomes a [`DoorEdge`] crossing the
/// wall: angle 0/2 crosses east-west, angle 1/3 north-south, and the door's
/// own tile is marked not walkable. Other blocking locs (unknown types
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
) -> Result<Mapsquare, PackError> {
    let text = std::fs::read_to_string(path).map_err(PackError::Io)?;
    parse_mapsquare_text(&text, mapsquare_x, mapsquare_z, door_ids, passable)
        .ok_or_else(|| PackError::BadLength(format!("{}: no MAP section", path.display())))
}

/// Parse jm2 text into a [`Mapsquare`], or None without a MAP section.
fn parse_mapsquare_text(
    text: &str,
    mapsquare_x: i32,
    mapsquare_z: i32,
    door_ids: &HashSet<i32>,
    passable: &HashSet<i32>,
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
            if let Some((x, z, blocked)) = parse_map_line(line) {
                walk[z * SQUARE + x] = if blocked { 0 } else { 1 };
            }
        } else if in_loc {
            if let Some(loc) = parse_loc_fields(line) {
                locs.push(loc);
            }
        }
    }
    if !saw_map {
        return None;
    }

    let mut doors = Vec::new();
    let mut door_sides = HashSet::new();
    for loc in &locs {
        if let Some(door) = door_edge(loc, mapsquare_x, mapsquare_z, door_ids) {
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

/// Parse a MAP line into `(x, z, blocked)`, level 0 only.
pub(crate) fn parse_map_line(line: &str) -> Option<(usize, usize, bool)> {
    let (coords, rest) = line.split_once(':')?;
    let mut c = coords.split_whitespace();
    let level: i32 = c.next()?.parse().ok()?;
    let x: i32 = c.next()?.parse().ok()?;
    let z: i32 = c.next()?.parse().ok()?;
    if c.next().is_some() {
        return None;
    }
    if level != 0 {
        return None;
    }
    let (x, z) = (x as usize, z as usize);
    if x >= SQUARE || z >= SQUARE {
        return None;
    }
    let blocked = rest.split_whitespace().any(|tok| {
        tok.strip_prefix('f')
            .and_then(|n| n.parse::<u32>().ok())
            .is_some_and(|flag| flag & 1 != 0)
    });
    Some((x, z, blocked))
}

/// One level-0 loc placement inside a mapsquare.
pub(crate) struct LocOnSquare {
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

/// Parse a LOC line into a level-0 placement.
pub(crate) fn parse_loc_fields(line: &str) -> Option<LocOnSquare> {
    let (coords, rest) = line.split_once(':')?;
    let mut c = coords.split_whitespace();
    let level: i32 = c.next()?.parse().ok()?;
    let x: i32 = c.next()?.parse().ok()?;
    let z: i32 = c.next()?.parse().ok()?;
    if c.next().is_some() {
        return None;
    }
    if level != 0 {
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
        x,
        z,
        loc_id,
        shape,
        angle,
    })
}

/// Shape-0 openable wall door -> DoorEdge, or None.
fn door_edge(
    loc: &LocOnSquare,
    mapsquare_x: i32,
    mapsquare_z: i32,
    door_ids: &HashSet<i32>,
) -> Option<DoorEdge> {
    if !door_ids.contains(&loc.loc_id) || loc.shape != 0 {
        return None;
    }
    let tile = Tile {
        x: mapsquare_x * SQUARE as i32 + loc.x as i32,
        z: mapsquare_z * SQUARE as i32 + loc.z as i32,
        level: 0,
    };
    let (from, to) = match loc.angle {
        0 | 2 => (
            Tile {
                x: tile.x - 1,
                z: tile.z,
                level: 0,
            },
            Tile {
                x: tile.x + 1,
                z: tile.z,
                level: 0,
            },
        ),
        1 | 3 => (
            Tile {
                x: tile.x,
                z: tile.z - 1,
                level: 0,
            },
            Tile {
                x: tile.x,
                z: tile.z + 1,
                level: 0,
            },
        ),
        _ => return None,
    };
    Some(DoorEdge {
        loc: tile,
        loc_id: loc.loc_id,
        from,
        to,
    })
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

fn read_u32(r: &mut Cursor<&[u8]>) -> Result<u32, PackError> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b).map_err(|_| PackError::Truncated)?;
    Ok(u32::from_le_bytes(b))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{
        decode, decode_v2, encode, encode_v2, merge_squares, parse_door_config,
        parse_mapsquare_text, parse_passable_locs, walkable_dots, Mapsquare,
    };
    use crate::collision::WorldCollision;
    use crate::grid::StepGrid;
    use crate::pack::PackError;
    use crate::tile::Tile;
    use crate::transport::{TransportEdge, TransportGraph, TransportKind};
    use api::snapshot::WorldTile;

    #[test]
    fn pack_roundtrip_fixture_door() {
        let g = StepGrid::fixture_door_corridor();
        let bytes = encode(&g);
        let h = decode(&bytes).unwrap();
        assert!(h.walkable(Tile {
            x: 0,
            z: 0,
            level: 0
        }));
        assert_eq!(h.doors.len(), g.doors.len());
    }

    #[test]
    fn v2_roundtrip_collision_and_transport_graph() {
        let flags = vec![0, 0, 1, 0, 0, 0];
        let collision = WorldCollision {
            origin: WorldTile {
                x: 3200,
                z: 3200,
                level: 0,
            },
            width: 3,
            height: 2,
            flags: flags.clone(),
            walkable: crate::collision::derive_walkable(&flags),
        };
        let mut graph = TransportGraph::default();
        let door = TransportEdge {
            kind: TransportKind::Door,
            from: WorldTile {
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
            skill_req: vec![],
            item_req: vec![],
            quest_req: vec![],
            varp_req: vec![],
        };
        let ladder = TransportEdge {
            kind: TransportKind::Ladder,
            from: WorldTile {
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
            skill_req: vec![(16, 5)],
            item_req: vec![(995, 10)],
            quest_req: vec!["Restless Ghost".into()],
            varp_req: vec![(4, 1)],
        };
        let di = graph.edges.len();
        graph.edges.push(door);
        let li = graph.edges.len();
        graph.edges.push(ladder);
        let glider = TransportEdge {
            kind: TransportKind::Glider,
            from: WorldTile {
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
            skill_req: vec![],
            item_req: vec![],
            quest_req: vec![],
            varp_req: vec![(150, 160)],
        };
        let gi = graph.edges.len();
        graph.edges.push(glider);
        // The any-tile teleport layer (Varrock spell): stored as a kind-4
        // edge in the same array, split back out on decode.
        graph.teleports.push(TransportEdge {
            kind: TransportKind::Teleport,
            from: WorldTile { x: 0, z: 0, level: 0 },
            to: WorldTile { x: 3213, z: 3424, level: 0 },
            loc_id: 0,
            option: 0,
            ticks: 3,
            skill_req: vec![(6, 25)],
            item_req: vec![(554, 1), (556, 3), (563, 1)],
            quest_req: vec![],
            varp_req: vec![],
        });
        graph.from.entry(graph.edges[di].from).or_default().push(di);
        graph.from.entry(graph.edges[li].from).or_default().push(li);
        graph.from.entry(graph.edges[gi].from).or_default().push(gi);

        let bytes = encode_v2(&collision, &graph);
        let (c, g) = decode_v2(&bytes).unwrap();
        assert_eq!(c.origin, collision.origin);
        assert_eq!(c.width, collision.width);
        assert_eq!(c.height, collision.height);
        assert_eq!(c.flags, collision.flags);
        assert_eq!(g.edges, graph.edges);
        // Teleports round-trip in their own layer, and the from-index is
        // rebuilt from the ordinary edges only.
        assert_eq!(g.teleports, graph.teleports);
        assert_eq!(g.from, graph.from);
        assert!(!g.from.contains_key(&WorldTile { x: 0, z: 0, level: 0 }));
        // The two formats do not cross-decode: v1 rejects v2 magic and
        // vice versa.
        assert!(matches!(decode(&bytes), Err(PackError::BadMagic)));
        assert!(matches!(decode_v2(&encode(&StepGrid::fixture_open_3x3())), Err(PackError::BadMagic)));
    }

    #[test]
    fn decode_rejects_bad_magic() {
        assert!(matches!(decode(b"XXXX"), Err(PackError::BadMagic)));
    }

    #[test]
    fn decode_rejects_truncated_pack() {
        let bytes = encode(&StepGrid::fixture_door_corridor());
        assert!(matches!(
            decode(&bytes[..bytes.len() - 1]),
            Err(PackError::Truncated)
        ));
    }

    #[test]
    fn decode_rejects_oversized_grid() {
        // Huge width would try to allocate GiB of walk bytes.
        let bytes = header(0, u32::MAX, 1);
        assert!(matches!(decode(&bytes), Err(PackError::BadLength(_))));
    }

    #[test]
    fn decode_rejects_zero_grid() {
        assert!(matches!(
            decode(&header(0, 0, 1)),
            Err(PackError::BadLength(_))
        ));
        assert!(matches!(
            decode(&header(0, 1, 0)),
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
        let sq = parse_mapsquare_text(text, 44, 53, &door_ids, &HashSet::new()).unwrap();
        // (0,0): no f flag -> walkable; (1,0): f1 bit 0 -> blocked;
        // (0,1): f16 bit 0 clear -> walkable; (2,0): no MAP line -> blocked.
        assert_eq!(sq.walk[0], 1);
        assert_eq!(sq.walk[1], 0);
        assert_eq!(sq.walk[64], 1);
        assert_eq!(sq.walk[2], 0);
        // The Catherby closed door: 1530 @ local (0,46) -> 2816,3438,0,
        // both from→to and to→from (same loc, loc_id).
        assert_eq!(sq.doors.len(), 2);
        let south = Tile {
            x: 2816,
            z: 3437,
            level: 0,
        };
        let north = Tile {
            x: 2816,
            z: 3439,
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
        let sq = parse_mapsquare_text(text, 44, 53, &HashSet::new(), &HashSet::new()).unwrap();
        let grid = merge_squares(&[sq]);
        assert!(!grid.walkable(Tile {
            x: 2816,
            z: 3437,
            level: 0
        }));
    }

    #[test]
    fn parse_jm2_does_not_wipe_door_from_to() {
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
        let sq = parse_mapsquare_text(text, 44, 53, &door_ids, &HashSet::new()).unwrap();
        let grid = merge_squares(&[sq]);
        assert!(!grid.walkable(Tile {
            x: 2816,
            z: 3438,
            level: 0
        }));
        assert!(grid.walkable(Tile {
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
}
