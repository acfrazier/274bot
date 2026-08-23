//! Nav pack: encode/decode of [`StepGrid`] to the `.navpack` binary format,
//! plus the jm2 mapsquare bake used by the `nav-pack` binary.
//!
//! Format: magic `b"274N"`, version `u8` 1, origin `(x, z, level)` i32le,
//! width/height u32le, one walk byte per tile (row-major z then x, 1 =
//! walkable, same indexing as [`StepGrid`]), door count u32le, then per door
//! `(loc_x, loc_z, loc_level, loc_id, from_x, from_z, from_level, to_x, to_z,
//! to_level)` all i32le. Door loc ids come from the Server
//! `content/scripts/doors/configs/*.loc` blocks (see [`parse_door_config`]).
//! Blocking loc footprints come from `[loc_N]` `blockwalk` (default yes).

use std::collections::HashSet;
use std::fmt;
use std::io::{self, Cursor, Read};
use std::path::Path;

use crate::grid::{DoorEdge, StepGrid};
use crate::tile::Tile;

/// Current pack format version.
const VERSION: u8 = 1;
/// File magic.
const MAGIC: &[u8; 4] = b"274N";
/// Mapsquare edge length in tiles.
const SQUARE: usize = 64;
/// Bytes per door entry.
const DOOR_BYTES: usize = 40;
/// Largest grid side a pack may decode (4096×4096 tiles).
const MAX_GRID: usize = 4096;

/// Errors loading, writing, or baking a nav pack.
#[derive(Debug)]
pub enum PackError {
    /// Filesystem read/write failure.
    Io(io::Error),
    /// File does not start with the `b"274N"` magic.
    BadMagic,
    /// Pack version is not [`VERSION`].
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
            PackError::BadMagic => write!(f, "bad pack magic (expected b\"274N\")"),
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
            doors.push(door);
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
fn section(line: &str) -> Option<&str> {
    line.strip_prefix("==== ")?.strip_suffix(" ====")
}

/// Parse a MAP line into `(x, z, blocked)`, level 0 only.
fn parse_map_line(line: &str) -> Option<(usize, usize, bool)> {
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
struct LocOnSquare {
    x: usize,
    z: usize,
    loc_id: i32,
    shape: i32,
    angle: i32,
}

/// Walls (0..=3), diagonal wall (9), and centrepiece (10, 11) occupy a walk
/// footprint. Ground decor (22) only blocks when active; wall decor (4..=8)
/// and roofs (12..=21) do not.
fn loc_blocks_tile(shape: i32) -> bool {
    matches!(shape, 0..=3 | 9..=11)
}

/// Parse a LOC line into a level-0 placement.
fn parse_loc_fields(line: &str) -> Option<LocOnSquare> {
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
        decode, encode, merge_squares, parse_door_config, parse_mapsquare_text,
        parse_passable_locs, walkable_dots, Mapsquare,
    };
    use crate::grid::StepGrid;
    use crate::pack::PackError;
    use crate::tile::Tile;

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
        // The Catherby closed door: 1530 @ local (0,46) -> 2816,3438,0.
        assert_eq!(sq.doors.len(), 1);
        let d = sq.doors[0];
        assert_eq!(
            d.loc,
            Tile {
                x: 2816,
                z: 3438,
                level: 0
            }
        );
        assert_eq!(d.loc_id, 1530);
        assert_eq!(
            d.from,
            Tile {
                x: 2816,
                z: 3437,
                level: 0
            }
        );
        assert_eq!(
            d.to,
            Tile {
                x: 2816,
                z: 3439,
                level: 0
            }
        );
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
