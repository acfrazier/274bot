//! Whole-world level-0 collision bake: every mapsquare's MAP f-flags and
//! LOC placements → one [`WorldCollision`] of `client::dash3d::CollisionFlag`
//! bitmasks, mirroring the client's `CollisionMap` stamping, plus the
//! derived per-tile walkable word ([`derive_walkable`]) the router's
//! directional step test reads.

use std::collections::HashSet;
use std::path::Path;

use api::obj_names::LocDefs;
use api::snapshot::WorldTile;
use client::dash3d::{CollisionFlag, LocAngle, LocShape};

use crate::pack::{parse_loc_fields, parse_map_line, section, PackError};

/// Mapsquare edge length in tiles.
const SQUARE: usize = 64;

/// Flags that make a level-0 tile unwalkable: any wall direction, a scenery
/// footprint, or MAP-blocked ground.
const WALK_BLOCK: u32 = CollisionFlag::WALK_BLOCK_FLAGS as u32
    | CollisionFlag::WALK_SCENERY as u32
    | CollisionFlag::WR_GRND as u32;

/// The client's `SQ_BLOCKED` base shared by every `PL_WALK_*` movement mask
/// (`WALK_SCENERY | BLOCK_NPCS_AND_PLAYERS | WR_GRND` = `0x280100`): any of
/// these on a tile makes the derived walkable word reject every direction.
const SQ_BLOCKED: u32 = CollisionFlag::WALK_SCENERY as u32
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

/// Whole-world level-0 collision: one `CollisionFlag` bitmask per tile,
/// row-major `z` then `x`, mirroring the client's `CollisionMap` build.
pub struct WorldCollision {
    /// The tile at `flags[0]`; the grid spans `width` tiles in +x then
    /// `height` rows in +z, all on level 0.
    pub origin: WorldTile,
    pub width: usize,
    pub height: usize,
    /// The raw baked word per tile: the client's `W_*`/`V_*` wall bits,
    /// `WALK_SCENERY` footprints, and `WR_GRND` ground blocks, exactly as
    /// `CollisionMap.add_wall`/`add_loc`/`block_ground` stamp them. Face
    /// flags alone do not reject every direction — see [`derive_walkable`].
    pub flags: Vec<u32>,
    /// The derived walkable word per tile, mirroring the client's movement
    /// masks: a raw wall bit `W_D` sets the full `PL_WALK_D` mask (which
    /// carries the shared `SQ_BLOCKED` base), so any wall flag — like
    /// blocked ground or scenery — makes the tile reject entry from every
    /// direction. The router's `step_ok` reads this word, never the raw
    /// `flags`.
    pub walkable: Vec<u32>,
}

impl WorldCollision {
    /// The collision bitmask at `(x, z, level)`, `0` for tiles outside the
    /// grid. The bake is one level-0 plane; other levels reuse that x,z cell.
    pub fn flag(&self, x: i32, z: i32, level: i32) -> u32 {
        // The bake is a single level-0 plane. Other levels reuse the same
        // x,z cell — returning 0 for off-level looked like "no flags", so
        // the router treated upstairs as an empty world and flooded it.
        let _ = level;
        let lx = x - self.origin.x;
        let lz = z - self.origin.z;
        if lx < 0 || lz < 0 {
            return 0;
        }
        let (lx, lz) = (lx as usize, lz as usize);
        if lx >= self.width || lz >= self.height {
            return 0;
        }
        self.flags[lz * self.width + lx]
    }

    /// The derived directional walkable word at `(x, z, level)`, `0` for
    /// tiles outside the grid (same indexing as [`Self::flag`]; off-level
    /// reuses the level-0 cell).
    pub fn walkable_word(&self, x: i32, z: i32, level: i32) -> u32 {
        let _ = level;
        let lx = x - self.origin.x;
        let lz = z - self.origin.z;
        if lx < 0 || lz < 0 {
            return 0;
        }
        let (lx, lz) = (lx as usize, lz as usize);
        if lx >= self.width || lz >= self.height {
            return 0;
        }
        self.walkable[lz * self.width + lx]
    }

    /// True when `t` sits on this bake's level-0 plane, inside its bounds,
    /// and has no walk-blocking flag. Tiles outside the grid are not
    /// walkable (the grid covers the whole world; beyond it is not a map).
    pub fn walkable(&self, t: WorldTile) -> bool {
        let lx = t.x - self.origin.x;
        let lz = t.z - self.origin.z;
        if lx < 0 || lz < 0 {
            return false;
        }
        let (lx, lz) = (lx as usize, lz as usize);
        if lx >= self.width || lz >= self.height {
            return false;
        }
        self.flags[lz * self.width + lx] & WALK_BLOCK == 0
    }

    /// True when `t` sits on this bake's level-0 plane, inside its bounds,
    /// and has no footprint block: no `WALK_SCENERY` footprint, no `WR_GRND`
    /// ground block, and no `SQ_BLOCKED` base. Directional face flags
    /// (`W_N`/`W_S`/`V_*`/…) do NOT disqualify standing — a wall's
    /// face-flagged floor tile can be stood on even though the router can
    /// never walk onto it. The transport interact-target neighbourhood is
    /// tested against this, never the stricter [`Self::walkable`].
    pub fn standable(&self, t: WorldTile) -> bool {
        let lx = t.x - self.origin.x;
        let lz = t.z - self.origin.z;
        if lx < 0 || lz < 0 {
            return false;
        }
        let (lx, lz) = (lx as usize, lz as usize);
        if lx >= self.width || lz >= self.height {
            return false;
        }
        self.flags[lz * self.width + lx]
            & (CollisionFlag::WALK_SCENERY as u32
                | CollisionFlag::WR_GRND as u32
                | CollisionFlag::SQ_BLOCKED as u32)
            == 0
    }

    /// The nearest [`Self::walkable`] tile at least one step from `t` along
    /// `(dx, dz)` — the door-edge snap: a door's blind ±1 `from`/`to` can
    /// land on a wall loc right outside the door (wall 980 south of the
    /// Catherby range-house door), and the router can no longer step onto
    /// that tile. A door at the edge of the bake keeps its blind ±1
    /// neighbour rather than walking off the grid.
    pub fn nearest_walkable(&self, t: WorldTile, dx: i32, dz: i32) -> WorldTile {
        let (mut x, mut z) = (t.x, t.z);
        loop {
            x += dx;
            z += dz;
            let inside = t.level == self.origin.level
                && x >= self.origin.x
                && z >= self.origin.z
                && (x - self.origin.x) < self.width as i32
                && (z - self.origin.z) < self.height as i32;
            if !inside {
                return WorldTile { x: t.x + dx, z: t.z + dz, level: t.level };
            }
            if self.walkable(WorldTile { x, z, level: t.level }) {
                return WorldTile { x, z, level: t.level };
            }
        }
    }
}

/// Bake the whole world: every `m<x>_<z>.jm2` under `maps_dir` (other files
/// are metadata, e.g. `ignore.csv`/`free2play.csv`, and are skipped). MAP
/// flags with bit 0 set stamp `WR_GRND`; LOC placements stamp flags by
/// shape/angle exactly like the client's `CollisionMap` (`add_wall` for
/// walls, `add_loc` footprints for scenery, `block_ground` for ground
/// decor). Openable doors (`door_ids`) are stamped blocked-when-closed.
/// All squares merge into one level-0 bounding grid.
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
    let min_x = squares.iter().map(|(x, _)| x * SQUARE as i32).min().unwrap();
    let min_z = squares.iter().map(|(_, z)| z * SQUARE as i32).min().unwrap();
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
    let mut flags = vec![0u32; width * height];

    for (square_x, square_z) in &squares {
        let path = maps_dir.join(format!("m{square_x}_{square_z}.jm2"));
        let text = std::fs::read_to_string(&path).map_err(PackError::Io)?;
        stamp_square(
            &mut flags,
            width,
            height,
            min_x,
            min_z,
            *square_x,
            *square_z,
            &text,
            loc_defs,
            door_ids,
        )?;
    }

    // Tiles no mapsquare covers (bbox gaps, non-rectangular world) are
    // blocked ground, like the walk-byte bake's un-stamped cells.
    let mut covered = vec![false; width * height];
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
    for (i, c) in covered.iter().enumerate() {
        if !c {
            flags[i] |= CollisionFlag::WR_GRND as u32;
        }
    }

    Ok(WorldCollision {
        origin: WorldTile {
            x: min_x,
            z: min_z,
            level: 0,
        },
        width,
        height,
        walkable: derive_walkable(&flags),
        flags,
    })
}

/// Derive the walkable word from the raw collision flags. Footprint and
/// ground (`SQ_BLOCKED`) reject every `tryMove` direction because those
/// masks share that base. Face flags (`W_N`/`W_S`/…) only reject the
/// matching face — they must not OR the full `PL_WALK_*` word, which
/// re-injects `SQ_BLOCKED` and seals open doorways (a Seers-bank stand
/// became a 31-tile pocket).
pub fn derive_walkable(flags: &[u32]) -> Vec<u32> {
    flags
        .iter()
        .map(|&raw| {
            let mut w = raw & SQ_BLOCKED;
            for &(bit, _mask) in &WALK_BITS {
                if raw & bit != 0 {
                    w |= bit;
                }
            }
            w
        })
        .collect()
}

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
            if let Some((x, z, blocked)) = parse_map_line(line) {
                if blocked {
                    set_at(
                        flags,
                        width,
                        height,
                        square_x * SQUARE as i32 + x as i32 - min_x,
                        square_z * SQUARE as i32 + z as i32 - min_z,
                        CollisionFlag::WR_GRND as u32,
                    );
                }
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

    for loc in &locs {
        // Local (origin-relative) tile coords; wall stamps may reach into
        // neighbouring squares of the same bbox.
        let (lx, lz) = (
            square_x * SQUARE as i32 + loc.x as i32 - min_x,
            square_z * SQUARE as i32 + loc.z as i32 - min_z,
        );
        let def = loc_defs.loc(loc.loc_id);
        // Unknown loc ids default to blocking, as the walk-byte bake did.
        let blockwalk = def.map_or(true, |d| d.block_walk);
        let blockrange = def.map_or(true, |d| d.block_range);
        // Openable wall doors are stamped blocked-when-closed, even though
        // the door-edge extraction is the transport task's.
        if door_ids.contains(&loc.loc_id) && loc.shape == LocShape::WALL_STRAIGHT {
            add_wall(flags, width, height, lx, lz, loc.shape, loc.angle, blockrange);
        } else if matches!(loc.shape, 0..=3) {
            if blockwalk {
                add_wall(flags, width, height, lx, lz, loc.shape, loc.angle, blockrange);
            }
        } else if loc.shape == LocShape::WALL_DIAGONAL
            || loc.shape == LocShape::CENTREPIECE_STRAIGHT
            || loc.shape == LocShape::CENTREPIECE_DIAGONAL
            || (LocShape::ROOF_STRAIGHT..=LocShape::ROOFEDGE_SQUARE_CORNER).contains(&loc.shape)
        {
            if blockwalk {
                let (w, l) = def.map_or((1, 1), |d| (d.width, d.length));
                add_loc(flags, width, height, lx, lz, w, l, loc.angle, blockrange);
            }
        } else if loc.shape == LocShape::GROUND_DECOR {
            // The client's `block_ground` (WR_GRND) is what blocks walk on
            // a blockwalk, active ground decor — the brief's original
            // WR_GROUND_DECOR semantic was wrong vs the client (that flag
            // is not in the walk mask).
            if blockwalk && def.map_or(false, |d| d.active) {
                set_at(flags, width, height, lx, lz, CollisionFlag::WR_GRND as u32);
            }
        }
        // Wall decor (4..=8) and unknown shapes carry no collision.
    }
    Ok(())
}

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
            set_at(flags, width, height, x, z, west);
            set_at(flags, width, height, x - 1, z, east);
        } else if angle == LocAngle::NORTH {
            set_at(flags, width, height, x, z, north);
            set_at(flags, width, height, x, z + 1, south);
        } else if angle == LocAngle::EAST {
            set_at(flags, width, height, x, z, east);
            set_at(flags, width, height, x + 1, z, west);
        } else if angle == LocAngle::SOUTH {
            set_at(flags, width, height, x, z, south);
            set_at(flags, width, height, x, z - 1, north);
        }
    } else if shape == LocShape::WALL_DIAGONAL_CORNER
        || shape == LocShape::WALL_SQUARE_CORNER
    {
        if angle == LocAngle::WEST {
            set_at(flags, width, height, x, z, north_west);
            set_at(flags, width, height, x - 1, z + 1, south_east);
        } else if angle == LocAngle::NORTH {
            set_at(flags, width, height, x, z, north_east);
            set_at(flags, width, height, x + 1, z + 1, south_west);
        } else if angle == LocAngle::EAST {
            set_at(flags, width, height, x, z, south_east);
            set_at(flags, width, height, x + 1, z - 1, north_west);
        } else if angle == LocAngle::SOUTH {
            set_at(flags, width, height, x, z, south_west);
            set_at(flags, width, height, x - 1, z - 1, north_east);
        }
    } else if shape == LocShape::WALL_L {
        if angle == LocAngle::WEST {
            set_at(flags, width, height, x, z, north | west);
            set_at(flags, width, height, x - 1, z, east);
            set_at(flags, width, height, x, z + 1, south);
        } else if angle == LocAngle::NORTH {
            set_at(flags, width, height, x, z, north | east);
            set_at(flags, width, height, x, z + 1, south);
            set_at(flags, width, height, x + 1, z, west);
        } else if angle == LocAngle::EAST {
            set_at(flags, width, height, x, z, south | east);
            set_at(flags, width, height, x + 1, z, west);
            set_at(flags, width, height, x, z - 1, north);
        } else if angle == LocAngle::SOUTH {
            set_at(flags, width, height, x, z, south | west);
            set_at(flags, width, height, x, z - 1, north);
            set_at(flags, width, height, x - 1, z, east);
        }
    }
    if blockrange {
        add_wall(flags, width, height, x, z, shape, angle, false);
    }
}

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
            set_at(flags, width, height, tx, tz, f);
        }
    }
}

/// OR `f` into the tile at local grid coords `(x, z)`, dropping tiles
/// outside the bbox (the client drops region-edge stamps the same way).
fn set_at(flags: &mut [u32], width: usize, height: usize, x: i32, z: i32, f: u32) {
    if x < 0 || z < 0 {
        return;
    }
    let (x, z) = (x as usize, z as usize);
    if x >= width || z >= height {
        return;
    }
    flags[z * width + x] |= f;
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::obj_names::LocDefs;
    use client::config::LocType;
    use std::fs;
    use std::path::PathBuf;

    /// A scratch directory for one fixture, removed on drop.
    struct FixtureDir(PathBuf);

    impl FixtureDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("274bot-nav-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            FixtureDir(dir)
        }
    }

    impl Drop for FixtureDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A one-loc `LocDefs` table.
    fn defs(locs: &[LocType]) -> LocDefs {
        LocDefs::from_locs(locs)
    }

    #[test]
    fn collision_bake_marks_wall_and_door_blocked() {
        let fix = FixtureDir::new("wall-and-door");
        let text = "\
==== MAP ====
0 0 0: h1 o6 u48
0 0 1: f1 u48
0 0 2: h1 o6 u50
==== LOC ====
0 0 2: 1530 0 1
";
        fs::write(fix.0.join("m50_50.jm2"), text).unwrap();
        let locs = defs(&[LocType {
            id: 1530,
            blockwalk: true,
            ..LocType::default()
        }]);
        let mut door_ids = HashSet::new();
        door_ids.insert(1530);
        let wc = bake_from_maps(&fix.0, &locs, &door_ids).unwrap();
        // m50_50 local (0,0) is absolute (3200, 3200).
        let open = WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        };
        let blocked = WorldTile {
            x: 3200,
            z: 3201,
            level: 0,
        };
        let door = WorldTile {
            x: 3200,
            z: 3202,
            level: 0,
        };
        assert!(wc.walkable(open));
        assert!(!wc.walkable(blocked));
        assert!(!wc.walkable(door));
        // f1 stamps WR_GRND; the closed door stamps the W_N wall flag.
        assert_eq!(
            wc.flag(blocked.x, blocked.z, blocked.level) & CollisionFlag::WR_GRND as u32,
            CollisionFlag::WR_GRND as u32
        );
        assert_eq!(
            wc.flag(door.x, door.z, door.level) & CollisionFlag::W_N as u32,
            CollisionFlag::W_N as u32
        );
        // Outside the grid: no flags, not walkable. Other levels reuse the
        // level-0 cell (empty off-level used to flood upstairs).
        assert_eq!(wc.flag(3199, 3200, 0), 0);
        assert!(wc.walkable(WorldTile {
            x: 3200,
            z: 3200,
            level: 1
        }));
        assert!(!wc.walkable(WorldTile {
            x: 3199,
            z: 3200,
            level: 0
        }));
    }

    #[test]
    fn bake_skips_non_jm2_files_and_merges_the_bbox() {
        let fix = FixtureDir::new("bbox-merge");
        fs::write(fix.0.join("ignore.csv"), "metadata\n").unwrap();
        fs::write(
            fix.0.join("m50_50.jm2"),
            "==== MAP ====\n0 0 0: h1 o6 u48\n",
        )
        .unwrap();
        fs::write(
            fix.0.join("m52_52.jm2"),
            "==== MAP ====\n0 0 0: h1 o6 u48\n",
        )
        .unwrap();
        let wc = bake_from_maps(&fix.0, &defs(&[]), &HashSet::new()).unwrap();
        // Origin is the western/northern corner; both walkable tiles are in.
        assert_eq!(wc.origin.x, 3200);
        assert_eq!(wc.origin.z, 3200);
        assert_eq!(wc.width, 3 * SQUARE as usize);
        assert_eq!(wc.height, 3 * SQUARE as usize);
        assert!(wc.walkable(WorldTile {
            x: 3200,
            z: 3200,
            level: 0
        }));
        assert!(wc.walkable(WorldTile {
            x: 3328,
            z: 3328,
            level: 0
        }));
        // The gap between the two squares stays blocked.
        assert!(!wc.walkable(WorldTile {
            x: 3264,
            z: 3264,
            level: 0
        }));
    }

    #[test]
    fn scenery_footprint_stamps_walk_scenery() {
        let fix = FixtureDir::new("scenery-footprint");
        let text = "\
==== MAP ====
0 0 0: h1 o6 u48
==== LOC ====
0 0 0: 1013 10 0
0 1 0: 602 10 1
";
        fs::write(fix.0.join("m50_50.jm2"), text).unwrap();
        let locs = defs(&[
            LocType {
                id: 1013,
                width: 2,
                length: 1,
                blockwalk: true,
                ..LocType::default()
            },
            LocType {
                id: 602,
                width: 2,
                length: 1,
                blockwalk: true,
                ..LocType::default()
            },
        ]);
        let wc = bake_from_maps(&fix.0, &locs, &HashSet::new()).unwrap();
        let t = |x: i32, z: i32| WorldTile { x, z, level: 0 };
        // Shape 10, angle 0 at (0,0): 2 wide in x -> (3200,3200) and
        // (3201,3200).
        assert!(!wc.walkable(t(3200, 3200)));
        // Shape 10, angle 1 (north) at (1,0): width/length swap -> 1 wide
        // in x, 2 long in z -> (3201,3200) and (3201,3201).
        assert!(!wc.walkable(t(3201, 3200)));
        assert!(!wc.walkable(t(3201, 3201)));
        // The tile west of both footprints is untouched and walkable.
        assert!(wc.walkable(t(3200, 3201)));
        assert_eq!(
            wc.flag(3201, 3200, 0) & CollisionFlag::WALK_SCENERY as u32,
            CollisionFlag::WALK_SCENERY as u32
        );
    }

    #[test]
    fn diagonal_wall_and_ground_decor_stamp_their_flags() {
        let fix = FixtureDir::new("diag-and-decor");
        let text = "\
==== MAP ====
0 0 0: h1 o6 u48
==== LOC ====
0 0 0: 1013 9 0
0 0 1: 1248 22 0
0 0 2: 559 22 0
";
        fs::write(fix.0.join("m50_50.jm2"), text).unwrap();
        let locs = defs(&[
            LocType {
                id: 1013,
                width: 1,
                length: 1,
                blockwalk: true,
                ..LocType::default()
            },
            LocType {
                id: 1248,
                blockwalk: true,
                active: true,
                ..LocType::default()
            },
            LocType {
                id: 559,
                blockwalk: true,
                active: false,
                ..LocType::default()
            },
        ]);
        let wc = bake_from_maps(&fix.0, &locs, &HashSet::new()).unwrap();
        let t = |x: i32, z: i32| WorldTile { x, z, level: 0 };
        // Shape 9 (wall diagonal) is a scenery-footprint loc like the
        // client: WALK_SCENERY on its tile.
        assert!(!wc.walkable(t(3200, 3200)));
        // A blockwalk && active ground decor blocks via the client's
        // `block_ground` (WR_GRND), which is in the walk mask.
        assert_eq!(
            wc.flag(3200, 3201, 0) & CollisionFlag::WR_GRND as u32,
            CollisionFlag::WR_GRND as u32
        );
        assert!(!wc.walkable(t(3200, 3201)));
        // The inactive ground decor loc (same shape, `active` gates it) is
        // not stamped and stays walkable.
        assert_eq!(wc.flag(3200, 3202, 0), 0);
        assert!(wc.walkable(t(3200, 3202)));
    }

    #[test]
    fn bake_rejects_mapsquare_without_a_map_section() {
        let fix = FixtureDir::new("no-map");
        fs::write(
            fix.0.join("m50_50.jm2"),
            "==== NPC ====\n0 0 0: 1234\n",
        )
        .unwrap();
        assert!(matches!(
            bake_from_maps(&fix.0, &defs(&[]), &HashSet::new()),
            Err(PackError::BadLength(_))
        ));
    }

    #[test]
    fn bake_rejects_empty_maps_dir() {
        let fix = FixtureDir::new("empty");
        assert!(matches!(
            bake_from_maps(&fix.0, &defs(&[]), &HashSet::new()),
            Err(PackError::BadLength(_))
        ));
    }

    /// A level-0 world at (3200,3200) with one flag word per tile.
    fn flag_world(flags: Vec<u32>) -> WorldCollision {
        let wc = WorldCollision {
            origin: WorldTile {
                x: 3200,
                z: 3200,
                level: 0,
            },
            width: flags.len(),
            height: 1,
            walkable: derive_walkable(&flags),
            flags,
        };
        wc
    }

    #[test]
    fn face_flag_tile_is_standable_but_not_walkable() {
        let wc = flag_world(vec![CollisionFlag::W_N as u32]);
        let t = WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        };
        // A directional face flag (the closed door's W_N) does not block
        // standing, but the blanket walkable() still rejects the tile.
        assert!(wc.standable(t));
        assert!(!wc.walkable(t));
    }

    #[test]
    fn footprint_and_ground_blocks_are_not_standable() {
        let wc = flag_world(vec![
            CollisionFlag::WR_GRND as u32,
            CollisionFlag::WALK_SCENERY as u32,
            0,
        ]);
        let t = |x: i32| WorldTile {
            x: 3200 + x,
            z: 3200,
            level: 0,
        };
        // A ground block and a scenery footprint disqualify standing just
        // like walking.
        assert!(!wc.standable(t(0)));
        assert!(!wc.walkable(t(0)));
        assert!(!wc.standable(t(1)));
        assert!(!wc.walkable(t(1)));
        // A clear tile stays both standable and walkable.
        assert!(wc.standable(t(2)));
        assert!(wc.walkable(t(2)));
    }

    #[test]
    fn range_face_flags_do_not_disqualify_standing() {
        // V_N (a blockrange wall's range stamp) is a face flag like W_N:
        // standable, never a footprint block.
        let wc = flag_world(vec![CollisionFlag::V_N as u32]);
        let t = WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        };
        assert!(wc.standable(t));
    }

    #[test]
    fn standable_rejects_out_of_grid() {
        let wc = flag_world(vec![0u32]);
        assert!(!wc.standable(WorldTile {
            x: 3199,
            z: 3200,
            level: 0
        }));
        // The bake is one level-0 plane; other levels reuse that x,z cell
        // (empty off-level used to look like "no flags" and flood upstairs).
        assert!(wc.standable(WorldTile {
            x: 3200,
            z: 3200,
            level: 1
        }));
    }

    #[test]
    fn off_level_reuses_the_level0_cell_instead_of_an_empty_plane() {
        let wc = flag_world(vec![CollisionFlag::WALK_SCENERY as u32]);
        let l0 = WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        };
        let l1 = WorldTile {
            x: 3200,
            z: 3200,
            level: 1,
        };
        assert_eq!(wc.flag(l1.x, l1.z, l1.level), wc.flag(l0.x, l0.z, l0.level));
        assert_eq!(
            wc.walkable_word(l1.x, l1.z, l1.level),
            wc.walkable_word(l0.x, l0.z, l0.level)
        );
        assert!(!wc.walkable(l1));
        assert!(!wc.standable(l1));
    }

    #[test]
    fn derive_walkable_does_not_seal_a_face_flag_from_every_side() {
        let wc = flag_world(vec![CollisionFlag::W_W as u32]);
        let t = WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        };
        let word = wc.walkable_word(t.x, t.z, t.level);
        assert_eq!(word & CollisionFlag::W_W as u32, CollisionFlag::W_W as u32);
        assert_eq!(
            word & SQ_BLOCKED,
            0,
            "a west wall must not inject SQ_BLOCKED"
        );
    }
}
