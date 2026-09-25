//! PNG decode for terrain tiles, plus test-only Lumbridge fixtures.
//! Production never encodes tiles or invents POIs.

use std::collections::BTreeMap;
use std::io::Cursor;

use nav::map::formats::MAX_PNG_BYTES;
use nav::map::identity::{Digest, ImageIdentity};
use nav::map::poi::PoiRecord;
use nav::map::spatial::{TileKey, MAX_LOD, TILE_PIXELS, TILE_RGBA_BYTES};
use nav::map::MapError;

#[cfg(test)]
use nav::map::formats::TileReceipt;
#[cfg(test)]
use nav::map::identity::CatalogueIdentity;
#[cfg(test)]
use nav::map::poi::{
    CapabilityEvidence, DisplayAnchor, EntityKind, Footprint, PoiKey, PoiKind, SourceSpace,
};
#[cfg(test)]
use nav::map::spatial::{TILE_GUTTER, TILE_INTERIOR};
#[cfg(test)]
use nav::map::{Rows, Text};

#[cfg(test)]
/// Lumbridge courtyard mapsquare group (64-tile LOD-0 cells).
pub const LUMB_LOD0: [(i32, i32); 4] = [(50, 50), (51, 50), (50, 51), (51, 51)];
#[cfg(test)]
pub const LUMB_LOD1: (i32, i32) = (25, 25);
#[cfg(test)]
pub const FIXTURE_MAX_LOD: u8 = 1;

#[cfg(test)]
pub fn image_identity() -> ImageIdentity {
    ImageIdentity {
        revision: 289,
        content: Digest([0xf1; 32]),
        policy: Digest([0xf2; 32]),
    }
}

#[cfg(test)]
#[allow(dead_code)]
pub fn catalogue_identity() -> CatalogueIdentity {
    CatalogueIdentity {
        revision: 289,
        content: Digest([0xf1; 32]),
        policy: Digest([0xf3; 32]),
    }
}

#[derive(Debug, Clone)]
pub struct Store {
    pub identity: ImageIdentity,
    pub max_lod: u8,
    tiles: BTreeMap<TileKey, Vec<u8>>,
    pub pois: Vec<PoiRecord>,
}

impl Store {
    /// Production catalogue: no tiles, no POIs, coarsest LOD allowed so
    /// `select_lod` can always fall back instead of erroring.
    pub fn empty() -> Self {
        Self {
            identity: ImageIdentity {
                revision: 0,
                content: Digest([0; 32]),
                policy: Digest([0; 32]),
            },
            max_lod: MAX_LOD,
            tiles: BTreeMap::new(),
            pois: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    #[cfg(test)]
    pub fn lumbridge() -> Result<Self, MapError> {
        let identity = image_identity();
        let mut tiles = BTreeMap::new();
        for &(x, z) in &LUMB_LOD0 {
            let key = TileKey {
                plane: 0,
                lod: 0,
                x,
                z,
            };
            tiles.insert(key, encode_tile(key)?);
        }
        let parent = TileKey {
            plane: 0,
            lod: 1,
            x: LUMB_LOD1.0,
            z: LUMB_LOD1.1,
        };
        tiles.insert(parent, encode_tile(parent)?);
        Ok(Self {
            identity,
            max_lod: FIXTURE_MAX_LOD,
            tiles,
            pois: fixture_pois()?,
        })
    }

    pub fn get(&self, key: TileKey) -> Option<&[u8]> {
        self.tiles.get(&key).map(Vec::as_slice)
    }
}

/// Encode a 258×258 RGBA8 noninterlaced PNG. Tests only.
#[cfg(test)]
pub fn encode_tile(key: TileKey) -> Result<Vec<u8>, MapError> {
    let rgba = raster_tile(key);
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, TILE_PIXELS, TILE_PIXELS);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder
            .write_header()
            .map_err(|_| MapError::Invalid("png encode header"))?;
        writer
            .write_image_data(&rgba)
            .map_err(|_| MapError::Invalid("png encode data"))?;
        writer
            .finish()
            .map_err(|_| MapError::Invalid("png encode finish"))?;
    }
    if out.len() as u32 > MAX_PNG_BYTES {
        return Err(MapError::Limit("png bytes"));
    }
    let receipt = TileReceipt {
        key,
        payload: nav::map::formats::PayloadReceipt {
            bytes: out.len() as u32,
            sha256: Digest::of(&out),
        },
    };
    receipt.verify_png(&out)?;
    Ok(out)
}

pub fn decode_tile(bytes: &[u8]) -> Result<Vec<u8>, MapError> {
    if bytes.len() < 33 {
        return Err(MapError::Truncated);
    }
    if &bytes[..8] != b"\x89PNG\r\n\x1a\n"
        || bytes[8..12] != 13u32.to_be_bytes()
        || &bytes[12..16] != b"IHDR"
    {
        return Err(MapError::Invalid("PNG header"));
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    if width != TILE_PIXELS || height != TILE_PIXELS || bytes[24..29] != [8, 6, 0, 0, 0] {
        return Err(MapError::Invalid("PNG dimensions/format"));
    }
    if bytes.len() as u32 > MAX_PNG_BYTES {
        return Err(MapError::Limit("png bytes"));
    }
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_limits(png::Limits { bytes: 512 * 1024 });
    let mut reader = decoder
        .read_info()
        .map_err(|_| MapError::Invalid("png decode info"))?;
    let size = reader
        .output_buffer_size()
        .ok_or(MapError::Limit("png output"))?;
    if size != TILE_RGBA_BYTES {
        return Err(MapError::Invalid("png output size"));
    }
    let mut rgba = vec![0u8; TILE_RGBA_BYTES];
    let info = reader
        .next_frame(&mut rgba)
        .map_err(|_| MapError::Invalid("png frame"))?;
    if info.width != TILE_PIXELS || info.height != TILE_PIXELS {
        return Err(MapError::Invalid("png frame size"));
    }
    Ok(rgba)
}

#[cfg(test)]
fn raster_tile(key: TileKey) -> Vec<u8> {
    let mut rgba = vec![0u8; TILE_RGBA_BYTES];
    let r0 = 40u8.wrapping_add((key.x as i16).unsigned_abs() as u8);
    let g0 = 80u8.wrapping_add((key.z as i16).unsigned_abs() as u8);
    let b0 = 30u8.wrapping_add(key.lod.wrapping_mul(40));
    for y in 0..TILE_PIXELS {
        for x in 0..TILE_PIXELS {
            let gutter = x < TILE_GUTTER
                || y < TILE_GUTTER
                || x >= TILE_GUTTER + TILE_INTERIOR
                || y >= TILE_GUTTER + TILE_INTERIOR;
            let north = y < TILE_GUTTER + 8;
            let i = ((y * TILE_PIXELS + x) * 4) as usize;
            let (r, g, b) = if gutter {
                (r0 / 2, g0 / 2, b0 / 2)
            } else if north {
                (r0, 220, b0)
            } else {
                (r0, g0, b0)
            };
            rgba[i] = r;
            rgba[i + 1] = g;
            rgba[i + 2] = b;
            rgba[i + 3] = 255;
        }
    }
    rgba
}

#[cfg(test)]
fn fixture_pois() -> Result<Vec<PoiRecord>, MapError> {
    let booth = PoiRecord {
        key: PoiKey {
            entity: EntityKind::Loc,
            id: 2213,
            x: 3208,
            z: 3218,
            source: SourceSpace::ClientVisual {
                plane: 0,
                link_below: false,
            },
            shape: 10,
            rotation: 0,
        },
        name: Text::new("Bank booth")?,
        kind: PoiKind::Bank,
        effective_plane: 0,
        footprint: Footprint {
            width: 1,
            length: 1,
        },
        display: DisplayAnchor {
            x: 3208.5,
            z: 3218.5,
            plane: 0,
        },
        evidence: Rows::new(vec![CapabilityEvidence::ActiveQuickBooth])?,
        walk_target: None,
    };
    booth.validate()?;
    let label = PoiRecord {
        key: PoiKey {
            entity: EntityKind::MapFunction,
            id: 5,
            x: 3222,
            z: 3218,
            source: SourceSpace::ClientVisual {
                plane: 0,
                link_below: false,
            },
            shape: 0,
            rotation: 0,
        },
        name: Text::new("Lumbridge")?,
        kind: PoiKind::MapSymbol { symbol: 5 },
        effective_plane: 0,
        footprint: Footprint {
            width: 1,
            length: 1,
        },
        display: DisplayAnchor {
            x: 3222.5,
            z: 3218.5,
            plane: 0,
        },
        evidence: Rows::new(vec![CapabilityEvidence::MapFunction { symbol: 5 }])?,
        walk_target: None,
    };
    label.validate()?;
    Ok(vec![booth, label])
}
