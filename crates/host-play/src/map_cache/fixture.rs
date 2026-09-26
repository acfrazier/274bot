//! Test fixtures for the map cache: a deterministic terrain PNG and a
//! producer that publishes a real (tiny) catalogue and two terrain tiles,
//! counting each artefact it produces. Built for this crate's tests and,
//! behind `test-support`, for front-end tests of map demand.

use std::sync::atomic::{AtomicUsize, Ordering};

use nav::map::cache::{BakeStage, UnitKey};
use nav::map::formats::{
    CatalogueManifest, ClientPois, ColorFormat, Coverage, CoverageLevel, ImageManifest,
    PayloadReceipt, PlaneBounds, TileReceipt,
};
use nav::map::identity::{Digest, CATALOGUE_SCHEMA, IMAGE_SCHEMA};
use nav::map::poi::PoiRecord;
use nav::map::spatial::{TileKey, WorldBounds, TILE_GUTTER, TILE_INTERIOR};
use nav::map::Rows;

use super::{
    ArtifactKind, BakeOutput, BakePlan, BakeRequest, BakeWriter, MapBakeProducer, MapCacheError,
    MapProfileDescriptor,
};

/// A 289 descriptor with a fixed content identity and fixed policies,
/// without a prepared client cache (the fixture producer needs none).
pub fn fixture_descriptor() -> MapProfileDescriptor {
    MapProfileDescriptor::new(289, Digest([7; 32]), Digest([8; 32]), Digest([9; 32]))
        .expect("fixture identities are valid")
}

/// Complete deterministic PNG: IHDR + a zlib stream containing one filtered
/// RGBA row per pixel. Real enough for A's PNG guard and the ReadyImages
/// reader; no mocked payload is published.
pub fn fixture_png(seed: u8) -> Vec<u8> {
    let width = 258u32;
    let height = 258u32;
    let row_bytes = width as usize * 4 + 1;
    let raw_len = row_bytes * height as usize;
    let mut raw = vec![0u8; raw_len];
    for y in 0..height as usize {
        raw[y * row_bytes] = 0;
        for x in 0..width as usize {
            let offset = y * row_bytes + 1 + x * 4;
            raw[offset] = seed;
            raw[offset + 1] = x as u8;
            raw[offset + 2] = y as u8;
            raw[offset + 3] = 255;
        }
    }
    let mut zlib = vec![0x78, 0x01];
    let mut index = 0;
    while index < raw.len() {
        let remaining = raw.len() - index;
        let chunk = remaining.min(65_535);
        let final_block = index + chunk == raw.len();
        zlib.push(u8::from(final_block));
        let length = chunk as u16;
        zlib.extend_from_slice(&length.to_le_bytes());
        zlib.extend_from_slice(&(!length).to_le_bytes());
        zlib.extend_from_slice(&raw[index..index + chunk]);
        index += chunk;
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    png_chunk(&mut out, b"IHDR", &ihdr);
    png_chunk(&mut out, b"IDAT", &zlib);
    png_chunk(&mut out, b"IEND", &[]);
    out
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for byte in bytes {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

fn png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], payload: &[u8]) {
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    out.extend_from_slice(&crc32(&[kind.as_slice(), payload].concat()).to_be_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// Publishes an empty catalogue and two terrain tiles per bake and counts
/// how many times each artefact was produced.
#[derive(Debug, Default)]
pub struct CountingMapProducer {
    catalogues: AtomicUsize,
    images: AtomicUsize,
}

impl CountingMapProducer {
    /// Catalogue artefacts produced (published or attempted).
    pub fn catalogue_runs(&self) -> usize {
        self.catalogues.load(Ordering::SeqCst)
    }

    /// Terrain-image artefacts produced: the local bake the operator pays for.
    pub fn image_runs(&self) -> usize {
        self.images.load(Ordering::SeqCst)
    }

    const TILES: [TileKey; 2] = [
        TileKey {
            plane: 0,
            lod: 0,
            x: 50,
            z: 50,
        },
        TileKey {
            plane: 0,
            lod: 1,
            x: 25,
            z: 25,
        },
    ];
}

impl MapBakeProducer for CountingMapProducer {
    fn plan(&self, request: &BakeRequest) -> Result<BakePlan, MapCacheError> {
        Ok(match request.artifact() {
            ArtifactKind::Catalogue => BakePlan {
                planned_units: 1,
                stage: BakeStage::Catalogue,
            },
            ArtifactKind::Images => BakePlan {
                planned_units: Self::TILES.len() as u32,
                stage: BakeStage::BaseTerrain,
            },
        })
    }

    fn run(
        &self,
        request: &BakeRequest,
        writer: &mut BakeWriter,
    ) -> Result<BakeOutput, MapCacheError> {
        match request.artifact() {
            ArtifactKind::Catalogue => {
                self.catalogues.fetch_add(1, Ordering::SeqCst);
                let identity = request.descriptor().catalogue_identity();
                let payload = ClientPois {
                    schema: CATALOGUE_SCHEMA,
                    identity,
                    coverage: Coverage {
                        npc_placements: CoverageLevel::Unavailable,
                        bank_services: CoverageLevel::Limited,
                        place_labels: CoverageLevel::Unavailable,
                        unresolved: Rows::new(Vec::new())?,
                    },
                    records: Rows::<PoiRecord, 4096>::new(Vec::new())?,
                }
                .encode()?;
                writer.publish_unit(UnitKey::ClientPois, &payload)?;
                Ok(BakeOutput::Catalogue(CatalogueManifest {
                    schema: CATALOGUE_SCHEMA,
                    identity,
                    key: identity.key()?,
                    record_count: 0,
                    payload: PayloadReceipt {
                        bytes: 1,
                        sha256: Digest([0; 32]),
                    },
                }))
            }
            ArtifactKind::Images => {
                self.images.fetch_add(1, Ordering::SeqCst);
                let mut tiles = Vec::with_capacity(Self::TILES.len());
                for (index, key) in Self::TILES.into_iter().enumerate() {
                    let png = fixture_png(index as u8 + 1);
                    writer.publish_unit(UnitKey::Terrain { tile: key }, &png)?;
                    tiles.push(TileReceipt {
                        key,
                        payload: PayloadReceipt {
                            bytes: png.len() as u32,
                            sha256: Digest::of(&png),
                        },
                    });
                }
                let identity = request.descriptor().image_identity();
                let bounds = WorldBounds {
                    west: 3200,
                    south: 3200,
                    east: 3328,
                    north: 3328,
                };
                Ok(BakeOutput::Images(ImageManifest {
                    schema: IMAGE_SCHEMA,
                    identity,
                    key: identity.key()?,
                    extent: bounds,
                    planes: Rows::new(vec![PlaneBounds { plane: 0, bounds }])?,
                    max_lod: 1,
                    interior: TILE_INTERIOR,
                    gutter: TILE_GUTTER,
                    color: ColorFormat::Rgba8Unorm,
                    tiles: Rows::new(tiles)?,
                }))
            }
        }
    }
}
