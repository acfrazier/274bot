use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use client::config::LocType;
use rs2vm::world::{Alloc, LocInfo, PlacedId, PlacedLoc, Tri, World, ZoneKey};
use rs2vm::{Coord, LocTypeId};

use super::index::{mapsquare_coords, parse_jm2_locs_ordered, Placement};

#[derive(Debug, thiserror::Error)]
pub(super) enum NavWorldError {
    #[error("read navigation maps directory {path}: {source}")]
    ReadMapsDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("read navigation map {path}: {source}")]
    ReadMap {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("read free-to-play map {path}: {source}")]
    ReadFreeMap {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid free-to-play row {line} in {path}: {value:?}")]
    InvalidFreeMapRow {
        path: PathBuf,
        line: usize,
        value: String,
    },
    #[error("map {path} refers to invalid location type {id}")]
    InvalidLocType { path: PathBuf, id: i32 },
    #[error("map {path} has invalid shape {shape} or angle {angle} for active loc {id}")]
    InvalidLocInfo {
        path: PathBuf,
        id: i32,
        shape: i32,
        angle: i32,
    },
    #[error("map {path} contains more active placements than fit in PlacedId")]
    PlacedIdOverflow { path: PathBuf },
}

/// A source placement and its engine object identity.
#[derive(Clone, Debug)]
pub(super) struct WorldPlacement {
    pub(super) placement: Placement,
    pub(super) placed_id: PlacedId,
}

/// The immutable base world supplied to each machine entry.
#[derive(Default)]
pub(super) struct NavWorld {
    placed: HashMap<PlacedId, PlacedLoc>,
    zones: HashMap<ZoneKey, Vec<PlacedLoc>>,
    mapsquares: HashSet<(i32, i32)>,
    free_zones: HashSet<ZoneKey>,
    by_type: HashMap<i32, Vec<WorldPlacement>>,
}
impl NavWorld {
    pub(super) fn load(
        maps_dir: &Path,
        content_root: &Path,
        loc_types: &[LocType],
    ) -> Result<Self, NavWorldError> {
        let mut world = Self {
            free_zones: read_free_zones(content_root)?,
            ..Self::default()
        };
        let entries =
            fs::read_dir(maps_dir).map_err(|source| NavWorldError::ReadMapsDirectory {
                path: maps_dir.to_path_buf(),
                source,
            })?;
        let mut maps = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| NavWorldError::ReadMapsDirectory {
                path: maps_dir.to_path_buf(),
                source,
            })?;
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let Some(coords) = mapsquare_coords(name) else {
                continue;
            };
            maps.push((path, coords));
        }
        maps.sort_by(|left, right| left.0.cmp(&right.0));

        let mut next_placed_id = 0u32;
        for (path, (mx, mz)) in maps {
            world.mapsquares.insert((mx, mz));
            let text = fs::read_to_string(&path).map_err(|source| NavWorldError::ReadMap {
                path: path.clone(),
                source,
            })?;
            let mut placements = parse_jm2_locs_ordered(&text, mx, mz);
            placements.sort_unstable_by_key(|(placement, raw_coord)| (placement.id, *raw_coord));
            for (placement, _) in placements {
                let Some(loc_type) = usize::try_from(placement.id)
                    .ok()
                    .and_then(|id| loc_types.get(id))
                else {
                    return Err(NavWorldError::InvalidLocType {
                        path,
                        id: placement.id,
                    });
                };
                if !loc_type.active {
                    continue;
                }
                let shape = u8::try_from(placement.shape).ok();
                let angle = u8::try_from(placement.angle).ok();
                let (Some(shape), Some(angle)) = (shape, angle) else {
                    return Err(NavWorldError::InvalidLocInfo {
                        path,
                        id: placement.id,
                        shape: placement.shape,
                        angle: placement.angle,
                    });
                };
                let id = PlacedId(next_placed_id);
                next_placed_id = next_placed_id
                    .checked_add(1)
                    .ok_or_else(|| NavWorldError::PlacedIdOverflow { path: path.clone() })?;
                let tile = Coord::new(placement.x, placement.level, placement.z);
                let placed = PlacedLoc {
                    id,
                    tile,
                    info: LocInfo {
                        ty: LocTypeId(placement.id),
                        shape,
                        angle,
                    },
                };
                world
                    .zones
                    .entry(ZoneKey::of(tile))
                    .or_default()
                    .push(placed);
                world.placed.insert(id, placed);
                world
                    .by_type
                    .entry(placement.id)
                    .or_default()
                    .push(WorldPlacement {
                        placement,
                        placed_id: id,
                    });
            }
        }
        Ok(world)
    }

    pub(super) fn placements(&self, loc_type: i32) -> &[WorldPlacement] {
        self.by_type
            .get(&loc_type)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub(super) fn mapsquare_count(&self) -> usize {
        self.mapsquares.len()
    }

    pub(super) fn zone_count(&self) -> usize {
        self.zones.len()
    }

    pub(super) fn placed_count(&self) -> usize {
        self.placed.len()
    }
}
impl World for NavWorld {
    fn zone_locs(&self, zone: ZoneKey) -> &[PlacedLoc] {
        self.zones.get(&zone).map(Vec::as_slice).unwrap_or(&[])
    }

    fn placed_loc(&self, id: PlacedId) -> Option<PlacedLoc> {
        self.placed.get(&id).cloned()
    }

    fn zone_alloc(&self, tile: Coord) -> Alloc {
        let x = tile.x();
        let z = tile.z();
        if !self.mapsquares.contains(&(x >> 6, z >> 6)) {
            return Alloc {
                on_members: Tri::No,
                on_free: Tri::No,
            };
        }
        Alloc {
            on_members: Tri::Yes,
            on_free: if self.free_zones.contains(&ZoneKey::of(tile)) {
                Tri::Yes
            } else {
                Tri::No
            },
        }
    }
}

fn read_free_zones(content_root: &Path) -> Result<HashSet<ZoneKey>, NavWorldError> {
    let path = content_root.join("maps/free2play.csv");
    let text = fs::read_to_string(&path).map_err(|source| NavWorldError::ReadFreeMap {
        path: path.clone(),
        source,
    })?;
    let mut zones = HashSet::new();
    for (index, raw) in text.lines().enumerate() {
        let row = raw.trim();
        if row.is_empty() || row.starts_with("//") {
            continue;
        }
        let mut parts = row.split('_');
        let parsed = (
            parts.next().and_then(|part| part.parse::<i32>().ok()),
            parts.next().and_then(|part| part.parse::<i32>().ok()),
            parts.next().and_then(|part| part.parse::<i32>().ok()),
            parts.next().and_then(|part| part.parse::<i32>().ok()),
            parts.next().and_then(|part| part.parse::<i32>().ok()),
        );
        let (Some(level), Some(mx), Some(mz), Some(lx), Some(lz)) = parsed else {
            return Err(NavWorldError::InvalidFreeMapRow {
                path,
                line: index + 1,
                value: row.to_owned(),
            });
        };
        if parts.next().is_some()
            || !(0..=3).contains(&level)
            || !(0..64).contains(&lx)
            || !(0..64).contains(&lz)
        {
            return Err(NavWorldError::InvalidFreeMapRow {
                path,
                line: index + 1,
                value: row.to_owned(),
            });
        }
        let Some(x) = mx.checked_mul(64).and_then(|base| base.checked_add(lx)) else {
            return Err(NavWorldError::InvalidFreeMapRow {
                path,
                line: index + 1,
                value: row.to_owned(),
            });
        };
        let Some(z) = mz.checked_mul(64).and_then(|base| base.checked_add(lz)) else {
            return Err(NavWorldError::InvalidFreeMapRow {
                path,
                line: index + 1,
                value: row.to_owned(),
            });
        };
        zones.insert(ZoneKey {
            x: x >> 3,
            z: z >> 3,
            level,
        });
    }
    Ok(zones)
}
