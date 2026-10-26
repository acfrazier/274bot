//! North-up, half-open world geometry. No allocation or world-sized iteration.
use super::MapError;
use serde::{Deserialize, Serialize};

pub const TILE_INTERIOR: u32 = 256;
pub const TILE_GUTTER: u32 = 1;
pub const TILE_PIXELS: u32 = TILE_INTERIOR + 2 * TILE_GUTTER;
pub const TILE_RGBA_BYTES: usize = (TILE_PIXELS * TILE_PIXELS * 4) as usize;
pub const BASE_WORLD_SPAN: i64 = 64;
pub const MAX_LOD: u8 = 26; // span 2^32: covers the signed world in at most four tiles.
pub const TEXTURE_CAP: usize = 24;
pub const SNAP_RADIUS: i32 = 16;
pub const INTERIOR_UV: [[f32; 2]; 2] = [[1.0 / 258.0; 2], [257.0 / 258.0; 2]];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameTile {
    pub x: i32,
    pub z: i32,
    pub plane: u8,
}
impl GameTile {
    pub fn validate(self) -> Result<(), MapError> {
        if self.plane >= 4 {
            return Err(MapError::Invalid("plane"));
        }
        Ok(())
    }
}
impl From<GameTile> for api::snapshot::WorldTile {
    fn from(t: GameTile) -> Self {
        Self {
            x: t.x,
            z: t.z,
            level: i32::from(t.plane),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldBounds {
    pub west: i32,
    pub south: i32,
    pub east: i32,
    pub north: i32,
}
impl WorldBounds {
    pub fn validate(self) -> Result<(), MapError> {
        if self.west >= self.east || self.south >= self.north {
            return Err(MapError::Invalid("bounds"));
        }
        Ok(())
    }
    pub fn contains(self, other: Self) -> bool {
        self.west <= other.west
            && self.south <= other.south
            && self.east >= other.east
            && self.north >= other.north
    }
    pub fn intersects(self, key: TileKey) -> Result<bool, MapError> {
        let [west, south, east, north] = key.bounds()?;
        Ok(west < i64::from(self.east)
            && south < i64::from(self.north)
            && east > i64::from(self.west)
            && north > i64::from(self.south))
    }
}

/// Canonical ordering is plane, lod, x, z; filenames are derived, never loaded
/// from an untrusted path string. Positive z is north (PNG row zero is north).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TileKey {
    pub plane: u8,
    pub lod: u8,
    pub x: i32,
    pub z: i32,
}
impl TileKey {
    pub fn bounds(self) -> Result<[i64; 4], MapError> {
        if self.plane >= 4 || self.lod > MAX_LOD {
            return Err(MapError::Invalid("tile plane/LOD"));
        }
        let span = BASE_WORLD_SPAN << self.lod;
        let west = i64::from(self.x) * span;
        let south = i64::from(self.z) * span;
        let east = west
            .checked_add(span)
            .ok_or(MapError::Invalid("tile bounds"))?;
        let north = south
            .checked_add(span)
            .ok_or(MapError::Invalid("tile bounds"))?;
        if east <= i64::from(i32::MIN)
            || north <= i64::from(i32::MIN)
            || west > i64::from(i32::MAX)
            || south > i64::from(i32::MAX)
        {
            return Err(MapError::Invalid("tile outside coordinate range"));
        }
        Ok([west, south, east, north])
    }
    pub fn relative_path(self) -> Result<std::path::PathBuf, MapError> {
        self.bounds()?;
        Ok(format!(
            "terrain/p{}/l{}/{}_{}.png",
            self.plane, self.lod, self.x, self.z
        )
        .into())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct View {
    pub west: f64,
    pub south: f64,
    pub east: f64,
    pub north: f64,
    /// Physical framebuffer pixels, not logical UI points.
    pub pixels_per_tile: f64,
    pub plane: u8,
    pub max_lod: u8,
}
impl View {
    pub fn validate(self) -> Result<(), MapError> {
        let values = [
            self.west,
            self.south,
            self.east,
            self.north,
            self.pixels_per_tile,
        ];
        if values.iter().any(|n| !n.is_finite())
            || self.west >= self.east
            || self.south >= self.north
            || self.pixels_per_tile <= 0.0
            || self.plane >= 4
            || self.max_lod > MAX_LOD
            || self.west < f64::from(i32::MIN)
            || self.south < f64::from(i32::MIN)
            || self.east > f64::from(i32::MAX)
            || self.north > f64::from(i32::MAX)
        {
            return Err(MapError::Invalid("view"));
        }
        Ok(())
    }
    pub fn world_to_screen(self, x: f64, z: f64) -> [f64; 2] {
        [
            (x - self.west) * self.pixels_per_tile,
            (self.north - z) * self.pixels_per_tile,
        ]
    }
    pub fn screen_to_world(self, x: f64, y: f64) -> [f64; 2] {
        [
            self.west + x / self.pixels_per_tile,
            self.north - y / self.pixels_per_tile,
        ]
    }
    /// Exact horizontal screen edges choose the cell immediately south; screen
    /// right/bottom are excluded. Negative coordinates use floor, not truncation.
    pub fn tile_at(self, x: f64, y: f64) -> Result<Option<GameTile>, MapError> {
        self.validate()?;
        if !x.is_finite() || !y.is_finite() {
            return Err(MapError::Invalid("screen point"));
        }
        if x < 0.0
            || y < 0.0
            || x >= (self.east - self.west) * self.pixels_per_tile
            || y >= (self.north - self.south) * self.pixels_per_tile
        {
            return Ok(None);
        }
        let [wx, wz] = self.screen_to_world(x, y);
        Ok(Some(GameTile {
            x: wx.floor() as i32,
            z: (wz.ceil() - 1.0) as i32,
            plane: self.plane,
        }))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisibleTiles {
    pub plane: u8,
    pub lod: u8,
    pub min_x: i32,
    pub min_z: i32,
    pub max_x: i32,
    pub max_z: i32,
}
impl VisibleTiles {
    pub fn len(self) -> u64 {
        (i64::from(self.max_x) - i64::from(self.min_x) + 1) as u64
            * (i64::from(self.max_z) - i64::from(self.min_z) + 1) as u64
    }
    pub fn is_empty(self) -> bool {
        self.min_x > self.max_x || self.min_z > self.max_z
    }
    pub fn keys(self) -> impl Iterator<Item = TileKey> {
        (self.min_x..=self.max_x).flat_map(move |x| {
            (self.min_z..=self.max_z).map(move |z| TileKey {
                plane: self.plane,
                lod: self.lod,
                x,
                z,
            })
        })
    }
}

/// Prefer native density (4 / 2^lod >= screen density), then coarsen to fit the
/// hard slot cap. Counts all intersecting groups conservatively, including gaps.
/// A view straddling both zero axes may require four slots even at MAX_LOD.
pub fn select_lod(view: View, cap: usize) -> Result<VisibleTiles, MapError> {
    view.validate()?;
    if cap == 0 || cap > TEXTURE_CAP {
        return Err(MapError::Limit("texture slots (1..=24)"));
    }
    let mut lod = 0;
    while lod < view.max_lod && 4.0 / ((1u64 << (lod + 1)) as f64) >= view.pixels_per_tile {
        lod += 1;
    }
    loop {
        let span = (BASE_WORLD_SPAN << lod) as f64;
        let tiles = VisibleTiles {
            plane: view.plane,
            lod,
            min_x: (view.west / span).floor() as i32,
            min_z: (view.south / span).floor() as i32,
            max_x: ((view.east / span).ceil() - 1.0) as i32,
            max_z: ((view.north / span).ceil() - 1.0) as i32,
        };
        if tiles.len() <= cap as u64 {
            return Ok(tiles);
        }
        if lod == view.max_lod {
            return Err(MapError::Limit("visible tiles at coarsest LOD"));
        }
        lod += 1;
    }
}

/// At most 33x33 calls. Ordered by Chebyshev, Manhattan, x, z. Never returns
/// the blocked request on a miss, changes plane, or scans the entire world.
pub fn snap_walkable(
    request: GameTile,
    mut walkable: impl FnMut(GameTile) -> bool,
) -> Option<GameTile> {
    if request.plane >= 4 {
        return None;
    }
    if walkable(request) {
        return Some(request);
    }
    type Rank = (i32, i32, i32, i32);
    let mut best: Option<(Rank, GameTile)> = None;
    for dx in -SNAP_RADIUS..=SNAP_RADIUS {
        for dz in -SNAP_RADIUS..=SNAP_RADIUS {
            if dx == 0 && dz == 0 {
                continue;
            }
            let (Some(x), Some(z)) = (request.x.checked_add(dx), request.z.checked_add(dz)) else {
                continue;
            };
            let tile = GameTile {
                x,
                z,
                plane: request.plane,
            };
            let rank = (dx.abs().max(dz.abs()), dx.abs() + dz.abs(), x, z);
            if best.as_ref().is_none_or(|(old, _)| rank < *old) && walkable(tile) {
                best = Some((rank, tile));
            }
        }
    }
    best.map(|(_, tile)| tile)
}
