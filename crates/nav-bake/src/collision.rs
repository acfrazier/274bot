//! Construct whole-world collision grids from source MAP and LOC sections.
//! The producer converts mapsquares into the planner's runtime collision
//! model; decoding and queries remain in `nav_planner::collision`.

use std::collections::HashSet;
use std::path::Path;

use api::obj_names::LocDefs;
use api::snapshot::WorldTile;
use client::dash3d::{CollisionFlag, LocAngle, LocShape};
use nav_planner::collision::{game_plane, pack_walk, WorldCollision, LEVELS};
use nav_planner::pack::PackError;

use crate::pack::{parse_loc_fields, parse_map_line, section, SQUARE};

/// Bake the whole world: every `m<x>_<z>.jm2` under `maps_dir` (other files
/// are metadata, e.g. `ignore.csv`/`free2play.csv`, and are skipped). MAP
/// flags with bit 0 set stamp `WR_GRND` on their LINK_BELOW-corrected
/// plane (the client's `finishBuild`); LOC placements stamp flags by
/// shape/angle exactly like the client's `CollisionMap` (`add_wall` for
/// walls, `add_loc` footprints for scenery — roofs included — and
/// `block_ground` for active ground decor), each on its own
/// LINK_BELOW-corrected plane (the client's `loadLocations` drops a loc
/// whose `mapl[1]` tile carries LINK_BELOW below the grid and shifts a
/// level-1 loc down to level 0 — the Lumbridge castle battlements stamp
/// this way). Openable doors (`door_ids`) are stamped blocked-when-closed.
/// All squares merge into one 4-plane bounding grid (levels 0..=3).
///
/// Any `.jm2` that cannot be read, has no MAP section, or has an
/// unrecognizable mapsquare name is an error: the whole world bakes or none
/// of it does.
pub fn bake_from_maps(
    maps_dir: &Path,
    loc_defs: &LocDefs,
    door_ids: &HashSet<i32>,
) -> Result<WorldCollision, crate::GenerationError> {
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
        return Err(PackError::BadLength("no m<x>_<z>.jm2 mapsquares found".into()).into());
    }
    squares.sort_unstable();

    // Bounding box in tiles (the existing merge_squares geometry).
    let min_x = squares
        .iter()
        .map(|(x, _)| x * SQUARE as i32)
        .min()
        .expect("Mapsquare list is nonempty after validation");
    let min_z = squares
        .iter()
        .map(|(_, z)| z * SQUARE as i32)
        .min()
        .expect("Mapsquare list is nonempty after validation");
    let max_x = squares
        .iter()
        .map(|(x, _)| (x + 1) * SQUARE as i32)
        .max()
        .expect("Mapsquare list is nonempty after validation");
    let max_z = squares
        .iter()
        .map(|(_, z)| (z + 1) * SQUARE as i32)
        .max()
        .expect("Mapsquare list is nonempty after validation");
    let (width, height) = ((max_x - min_x) as usize, (max_z - min_z) as usize);
    let plane_cells = width * height;
    let mut flags = vec![0u32; LEVELS * plane_cells];

    for (square_x, square_z) in &squares {
        let path = maps_dir.join(format!("m{square_x}_{square_z}.jm2"));
        let text = std::fs::read_to_string(&path).map_err(PackError::Io)?;
        stamp_square(
            &mut flags, width, height, min_x, min_z, *square_x, *square_z, &text, loc_defs,
            door_ids,
        )?;
    }

    // Tiles no mapsquare covers (bbox gaps, non-rectangular world) are
    // blocked ground. Level 0 is the ground plane and is always blocked at
    // the gaps; an upper plane is only blocked at the gaps when the bake
    // actually stamped it — an unused plane stays empty (walkable) so gap
    // fill never fabricates it into an "existing" level.
    let mut covered = vec![false; plane_cells];
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
    for level in 0..LEVELS {
        let base = level * plane_cells;
        if level > 0 && !flags[base..base + plane_cells].iter().any(|&f| f != 0) {
            continue;
        }
        for (i, c) in covered.iter().enumerate() {
            if !c {
                flags[base + i] |= CollisionFlag::WR_GRND as u32;
            }
        }
    }

    let (walk, blocked) = pack_walk(&flags);
    Ok(WorldCollision {
        origin: WorldTile {
            x: min_x,
            z: min_z,
            level: 0,
        },
        width,
        height,
        walk,
        blocked,
        flags: Some(flags),
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "Stamping combines grid geometry, mapsquare data, loc definitions, and doors."
)]
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
    let mut map_rows: Vec<(i32, usize, usize, u32)> = Vec::new();
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
            if let Some(row) = parse_map_line(line) {
                map_rows.push(row);
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

    // The client's `finishBuild` reads the LINK_BELOW bit from the level-1
    // map flags (`mapl[1]`) for every level's BLOCK tiles, so the MAP pass
    // is two-stage: collect the rows, then stamp with the corrected level.
    let link_below: HashSet<(usize, usize)> = map_rows
        .iter()
        .filter(|&&(level, _, _, f)| level == 1 && f & 2 != 0)
        .map(|&(_, x, z, _)| (x, z))
        .collect();
    for &(level, x, z, f) in &map_rows {
        if f & 1 == 0 {
            continue;
        }
        // Client `finishBuild` (TS 76-90): a BLOCK tile blocks its
        // LINK_BELOW-corrected plane; `true_level < 0` is never stamped.
        if let Some(true_level) = game_plane(level, link_below.contains(&(x, z))) {
            set_at(
                flags,
                width,
                height,
                square_x * SQUARE as i32 + x as i32 - min_x,
                square_z * SQUARE as i32 + z as i32 - min_z,
                true_level,
                CollisionFlag::WR_GRND as u32,
            );
        }
    }

    for loc in &locs {
        // Client `loadLocations` (ClientBuild.ts): a loc whose tile has
        // LINK_BELOW on the level-1 map flags is placed on `level - 1`;
        // `level - 1 < 0` drops the loc entirely (no collision plane).
        let Some(true_level) = game_plane(loc.level, link_below.contains(&(loc.x, loc.z))) else {
            continue;
        };
        // Local (origin-relative) tile coords; wall stamps may reach into
        // neighbouring squares of the same bbox.
        let (lx, lz) = (
            square_x * SQUARE as i32 + loc.x as i32 - min_x,
            square_z * SQUARE as i32 + loc.z as i32 - min_z,
        );
        let def = loc_defs.loc(loc.loc_id);
        // Unknown loc ids default to blocking, as the walk-byte bake did.
        let blockwalk = def.is_none_or(|d| d.block_walk);
        let blockrange = def.is_none_or(|d| d.block_range);
        // Openable wall doors are stamped blocked-when-closed, even though
        // the door-edge extraction is the transport task's. Everything
        // stamps its own (LINK_BELOW-corrected) level plane, never a
        // level-0 squash.
        if door_ids.contains(&loc.loc_id) && loc.shape == LocShape::WALL_STRAIGHT {
            add_wall(
                flags, width, height, lx, lz, loc.shape, loc.angle, blockrange, true_level,
            );
        } else {
            // The client `addLoc` collision table (build.rs): ground decor
            // uses `block_ground`, centrepieces/roofs/wall-diagonal use
            // `add_loc`, walls use `add_wall`, wall decor carries nothing.
            match loc.shape {
                LocShape::GROUND_DECOR => {
                    // The client's `block_ground` (WR_GRND) is what blocks
                    // walk on a blockwalk, active ground decor.
                    if blockwalk && def.is_some_and(|d| d.active) {
                        set_at(
                            flags,
                            width,
                            height,
                            lx,
                            lz,
                            true_level,
                            CollisionFlag::WR_GRND as u32,
                        );
                    }
                }
                LocShape::CENTREPIECE_STRAIGHT
                | LocShape::CENTREPIECE_DIAGONAL
                | LocShape::WALL_DIAGONAL
                | LocShape::ROOF_STRAIGHT..=LocShape::ROOFEDGE_SQUARE_CORNER => {
                    if blockwalk {
                        let (w, l) = def.map_or((1, 1), |d| (d.width, d.length));
                        add_loc(
                            flags, width, height, lx, lz, w, l, loc.angle, blockrange, true_level,
                        );
                    }
                }
                LocShape::WALL_STRAIGHT
                | LocShape::WALL_DIAGONAL_CORNER
                | LocShape::WALL_L
                | LocShape::WALL_SQUARE_CORNER
                    if blockwalk =>
                {
                    add_wall(
                        flags, width, height, lx, lz, loc.shape, loc.angle, blockrange, true_level,
                    );
                }
                // Wall decor (4..=8) and unknown shapes carry no collision.
                _ => {}
            }
        }
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "Mirrors the client's wall-stamping inputs without changing their meaning."
)]
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
    level: i32,
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
            set_at(flags, width, height, x, z, level, west);
            set_at(flags, width, height, x - 1, z, level, east);
        } else if angle == LocAngle::NORTH {
            set_at(flags, width, height, x, z, level, north);
            set_at(flags, width, height, x, z + 1, level, south);
        } else if angle == LocAngle::EAST {
            set_at(flags, width, height, x, z, level, east);
            set_at(flags, width, height, x + 1, z, level, west);
        } else if angle == LocAngle::SOUTH {
            set_at(flags, width, height, x, z, level, south);
            set_at(flags, width, height, x, z - 1, level, north);
        }
    } else if shape == LocShape::WALL_DIAGONAL_CORNER || shape == LocShape::WALL_SQUARE_CORNER {
        if angle == LocAngle::WEST {
            set_at(flags, width, height, x, z, level, north_west);
            set_at(flags, width, height, x - 1, z + 1, level, south_east);
        } else if angle == LocAngle::NORTH {
            set_at(flags, width, height, x, z, level, north_east);
            set_at(flags, width, height, x + 1, z + 1, level, south_west);
        } else if angle == LocAngle::EAST {
            set_at(flags, width, height, x, z, level, south_east);
            set_at(flags, width, height, x + 1, z - 1, level, north_west);
        } else if angle == LocAngle::SOUTH {
            set_at(flags, width, height, x, z, level, south_west);
            set_at(flags, width, height, x - 1, z - 1, level, north_east);
        }
    } else if shape == LocShape::WALL_L {
        if angle == LocAngle::WEST {
            set_at(flags, width, height, x, z, level, north | west);
            set_at(flags, width, height, x - 1, z, level, east);
            set_at(flags, width, height, x, z + 1, level, south);
        } else if angle == LocAngle::NORTH {
            set_at(flags, width, height, x, z, level, north | east);
            set_at(flags, width, height, x, z + 1, level, south);
            set_at(flags, width, height, x + 1, z, level, west);
        } else if angle == LocAngle::EAST {
            set_at(flags, width, height, x, z, level, south | east);
            set_at(flags, width, height, x + 1, z, level, west);
            set_at(flags, width, height, x, z - 1, level, north);
        } else if angle == LocAngle::SOUTH {
            set_at(flags, width, height, x, z, level, south | west);
            set_at(flags, width, height, x, z - 1, level, north);
            set_at(flags, width, height, x - 1, z, level, east);
        }
    }
    if blockrange {
        add_wall(flags, width, height, x, z, shape, angle, false, level);
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "Mirrors the client's loc-stamping inputs without changing their meaning."
)]
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
    level: i32,
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
            set_at(flags, width, height, tx, tz, level, f);
        }
    }
}

/// OR `f` into the tile at local grid coords `(x, z, level)`, dropping
/// tiles outside the bbox or on unknown levels (the client drops region-
/// edge stamps the same way).
fn set_at(flags: &mut [u32], width: usize, height: usize, x: i32, z: i32, level: i32, f: u32) {
    if level < 0 || level >= LEVELS as i32 {
        return;
    }
    if x < 0 || z < 0 {
        return;
    }
    let (x, z) = (x as usize, z as usize);
    if x >= width || z >= height {
        return;
    }
    flags[level as usize * (width * height) + z * width + x] |= f;
}

#[cfg(test)]
#[path = "collision_tests.rs"]
mod tests;
