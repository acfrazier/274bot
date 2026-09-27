use super::*;

/// Mapsquare edge length in tiles.
pub(super) const SQUARE: usize = 64;

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
pub(super) fn parse_mapsquare_text(
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
pub(super) fn loc_blocks_tile(shape: i32) -> bool {
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
pub(super) fn door_edge(
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
pub(super) fn to_tile(t: WorldTile) -> Tile {
    Tile {
        x: t.x,
        z: t.z,
        level: t.level,
    }
}

/// Absolute tile -> local mapsquare coords, if it sits in that square.
pub(super) fn local_in_square(
    t: Tile,
    mapsquare_x: i32,
    mapsquare_z: i32,
) -> Option<(usize, usize)> {
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
