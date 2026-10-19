//! Test fixtures for the map cache: a deterministic terrain PNG, a producer
//! that publishes a real (tiny) catalogue and two terrain tiles, counting
//! each artefact it produces, that terrain shipped the way release packaging
//! ships it, and a minimal client snapshot for the real raster. Built for
//! this crate's tests and, behind `test-support`, for front-end tests of map
//! demand.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use api::work::{Checkpoint as WorkCheckpoint, MapBakePhase, StepContext, WorkCharge};
use nav::map::cache::{BakeStage, CompletedUnit, UnitKey};
use nav::map::formats::{
    ClientPois, ColorFormat, Coverage, CoverageLevel, ImageManifest, PlaneBounds, TileReceipt,
};
use nav::map::identity::{Digest, CATALOGUE_SCHEMA, IMAGE_SCHEMA};
use nav::map::poi::PoiRecord;
use nav::map::spatial::{TileKey, WorldBounds, TILE_GUTTER, TILE_INTERIOR};
use nav::map::Rows;

use super::{
    ArtifactKind, BakePlan, BakeRequest, BakeWriter, MapBakeCursor, MapBakeCursorOutput,
    MapBakeCursorStep, MapBakeProducer, MapCacheError, MapCacheRoot, MapDemand, MapDemandHandle,
    MapDemandManager, MapJobStatus, MapProfileDescriptor, MapProgressSnapshot, MapStage,
    ShippedMap,
};

/// A 289 descriptor with a fixed content identity and fixed policies,
/// without a prepared client cache (the fixture producer needs none).
pub fn fixture_descriptor() -> MapProfileDescriptor {
    MapProfileDescriptor::new(289, Digest([7; 32]), Digest([8; 32]), Digest([9; 32]))
        .expect("fixture identities are valid")
}

/// [`fixture_descriptor`] for another client cache: same revision and
/// policies, different decoded content.
pub fn other_cache_descriptor() -> MapProfileDescriptor {
    MapProfileDescriptor::new(289, Digest([6; 32]), Digest([8; 32]), Digest([9; 32]))
        .expect("fixture identities are valid")
}
// Hang guard only, not a pass condition and not a latency target. Every map
// job reaches a terminal status and a demand refused as Full is resubmitted on
// each `status()` turn, so the wait below ends on the product's own signal.
// The process-wide WorkService admits at most eight operation handles and a
// map job charges at least four (`host_map_bake_charge`), so at most two map
// jobs run at once across every test in the process; a demand waits behind the
// other tests' jobs and their durable artifact writes, and on a loaded host
// that queue is longer than any single job. The guard therefore only stops a
// deadlocked test and is minutes, not the expected duration.
const MAP_FIXTURE_SETTLEMENT_TIMEOUT: Duration = Duration::from_secs(600);

/// Wait for an expected terminal status through the production publication port.
pub fn wait_for_status(handle: &MapDemandHandle, expected: MapJobStatus) {
    let started = Instant::now();
    loop {
        let status = handle.status();
        if status == expected {
            return;
        }
        assert!(
            !super::is_terminal(&status),
            "map demand ended {status:?}, expected {expected:?}"
        );
        assert!(
            started.elapsed() < MAP_FIXTURE_SETTLEMENT_TIMEOUT,
            "map demand never settled: {status:?}, expected {expected:?}"
        );
        handle.wait_for_change(Duration::from_millis(100));
    }
}

/// [`fixture_descriptor`]'s terrain shipped under `dir/map` the way release
/// packaging ships a bake: the counting producer publishes it into a scratch
/// cache under `dir`, which is shipped and then removed.
pub fn ship_fixture_terrain(dir: &Path) -> ShippedMap {
    let scratch = dir.join("packaging-cache");
    let root = MapCacheRoot::from_root(&scratch);
    let manager = MapDemandManager::new(root.clone(), Arc::new(CountingMapProducer::default()));
    let handle = manager
        .request(fixture_descriptor(), MapDemand::Images)
        .expect("fixture terrain demand");
    wait_for_status(&handle, MapJobStatus::Ready);
    drop(handle);
    manager.reap();
    let shipped = ShippedMap::at(dir.join("map"));
    shipped
        .ship_images_from(&root, fixture_descriptor().image_identity())
        .expect("ship fixture terrain");
    let _ = std::fs::remove_dir_all(&scratch);
    shipped
}

/// [`fixture_descriptor`]'s catalogue shipped under `dir/map` the way release
/// packaging ships a derive: the counting producer publishes it into a scratch
/// cache under `dir`, which is shipped and then removed.
pub fn ship_fixture_catalogue(dir: &Path) -> ShippedMap {
    let scratch = dir.join("packaging-cache");
    let root = MapCacheRoot::from_root(&scratch);
    let manager = MapDemandManager::new(root.clone(), Arc::new(CountingMapProducer::default()));
    let handle = manager
        .request(fixture_descriptor(), MapDemand::CatalogueOnly)
        .expect("fixture catalogue demand");
    wait_for_status(&handle, MapJobStatus::Ready);
    drop(handle);
    manager.reap();
    let shipped = ShippedMap::at(dir.join("map"));
    shipped
        .ship_catalogue_from(&root, fixture_descriptor().catalogue_identity())
        .expect("ship fixture catalogue");
    let _ = std::fs::remove_dir_all(&scratch);
    shipped
}

/// A minimal client snapshot for the real raster: one floor, one loc
/// definition, no sprites, and three plane-0 mapsquares of that floor.
/// Returns its jag and snapshot directories under `dir`.
pub fn synthetic_client_snapshot(dir: &Path) -> (PathBuf, PathBuf) {
    use client::io::cache_289::synthetic_jag;
    let (jag, snapshot) = (dir.join("jag"), dir.join("snapshot"));
    std::fs::create_dir_all(&jag).unwrap();
    std::fs::create_dir_all(&snapshot).unwrap();
    let config: [(&str, &[u8]); 3] = [
        ("flo.dat", &[0, 1, 1, 0x40, 0x80, 0x20, 0]),
        ("loc.dat", &[0, 1, 0]),
        ("loc.idx", &[0, 1, 0, 1]),
    ];
    std::fs::write(jag.join("config"), synthetic_jag(&config)).unwrap();
    for sprites in ["media", "textures"] {
        std::fs::write(jag.join(sprites), synthetic_jag(&[("index.dat", &[0, 0])])).unwrap();
    }
    // Plane 0: underlay 1 (opcode 82) with a varied explicit height;
    // planes 1-3 empty.
    let mut land = Vec::new();
    for plane in 0..4 {
        for cell in 0..64 * 64u32 {
            if plane == 0 {
                land.extend([82, 1, (cell % 23) as u8 + 2]);
            } else {
                land.push(0);
            }
        }
    }
    let (mut index, mut maps) = (Vec::new(), Vec::new());
    for (file, x) in [50u16, 51, 52].into_iter().enumerate() {
        index.extend((x << 8 | 50).to_be_bytes());
        index.extend((file as u16).to_be_bytes());
        index.extend(u16::MAX.to_be_bytes());
        index.push(0);
        maps.extend((file as u32).to_le_bytes());
        maps.extend((land.len() as u32).to_le_bytes());
        maps.extend(&land);
    }
    std::fs::write(
        jag.join("versionlist"),
        synthetic_jag(&[("map_index", index.as_slice())]),
    )
    .unwrap();
    std::fs::write(snapshot.join("maps.bin"), maps).unwrap();
    (jag, snapshot)
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
/// each artefact it produces.
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

struct CountingCursor {
    artifact: ArtifactKind,
    catalogue_identity: nav::map::identity::CatalogueIdentity,
    image_identity: nav::map::identity::ImageIdentity,
    partial_dir: Arc<PathBuf>,
    plan: BakePlan,
    progress: MapProgressSnapshot,
    next_tile: usize,
    tiles: Vec<TileReceipt>,
    completed: Vec<CompletedUnit>,
}

impl CountingCursor {
    fn new(request: &BakeRequest, partial_dir: Arc<PathBuf>) -> Self {
        let artifact = request.artifact();
        let (plan, progress) = match artifact {
            ArtifactKind::Catalogue => (
                BakePlan {
                    planned_units: 1,
                    stage: BakeStage::Catalogue,
                },
                MapProgressSnapshot {
                    stage: MapStage::DerivingPois,
                    completed: 0,
                    total: Some(1),
                },
            ),
            ArtifactKind::Images => (
                BakePlan {
                    planned_units: CountingMapProducer::TILES.len() as u32,
                    stage: BakeStage::BaseTerrain,
                },
                MapProgressSnapshot {
                    stage: MapStage::BakingPlane,
                    completed: 0,
                    total: Some(CountingMapProducer::TILES.len() as u32),
                },
            ),
        };
        Self {
            artifact,
            catalogue_identity: request.descriptor().catalogue_identity(),
            image_identity: request.descriptor().image_identity(),
            partial_dir,
            plan,
            progress,
            next_tile: 0,
            tiles: Vec::with_capacity(CountingMapProducer::TILES.len()),
            completed: Vec::with_capacity(1),
        }
    }

    fn catalogue(&self) -> Result<ClientPois, MapCacheError> {
        Ok(ClientPois {
            schema: CATALOGUE_SCHEMA,
            identity: self.catalogue_identity,
            coverage: Coverage {
                npc_placements: CoverageLevel::Unavailable,
                bank_services: CoverageLevel::Limited,
                place_labels: CoverageLevel::Unavailable,
                unresolved: Rows::new(Vec::new())?,
            },
            records: Rows::<PoiRecord, 4096>::new(Vec::new())?,
        })
    }

    fn manifest(&mut self) -> Result<ImageManifest, MapCacheError> {
        let bounds = WorldBounds {
            west: 3200,
            south: 3200,
            east: 3328,
            north: 3328,
        };
        Ok(ImageManifest {
            schema: IMAGE_SCHEMA,
            identity: self.image_identity,
            key: self.image_identity.key()?,
            extent: bounds,
            planes: Rows::new(vec![PlaneBounds { plane: 0, bounds }])?,
            max_lod: 1,
            interior: TILE_INTERIOR,
            gutter: TILE_GUTTER,
            color: ColorFormat::Rgba8Unorm,
            tiles: Rows::new(std::mem::take(&mut self.tiles))?,
        })
    }
}

impl MapBakeCursor for CountingCursor {
    fn plan(&self) -> Option<BakePlan> {
        Some(self.plan)
    }

    fn step(
        &mut self,
        context: &mut StepContext<'_, MapBakePhase>,
        writer: Option<&mut BakeWriter>,
    ) -> Result<MapBakeCursorStep, MapCacheError> {
        match context.checkpoint() {
            WorkCheckpoint::Cancel(_) => return Ok(MapBakeCursorStep::Cancelled),
            WorkCheckpoint::Yield => return Ok(MapBakeCursorStep::Yielded),
            WorkCheckpoint::Continue => {}
        }
        match self.artifact {
            ArtifactKind::Catalogue => {
                let pois = self.catalogue()?;
                self.progress.completed = 1;
                if matches!(context.consume_unit(), WorkCheckpoint::Cancel(_)) {
                    return Ok(MapBakeCursorStep::Cancelled);
                }
                Ok(MapBakeCursorStep::Completed(
                    MapBakeCursorOutput::Catalogue(pois),
                ))
            }
            ArtifactKind::Images => {
                let Some(writer) = writer else {
                    return Ok(MapBakeCursorStep::Yielded);
                };
                if writer.directory() != self.partial_dir.as_path() {
                    return Err(MapCacheError::Map(nav::map::MapError::Path));
                }
                let key = CountingMapProducer::TILES[self.next_tile];
                let unit = UnitKey::Terrain { tile: key };
                let receipt = match writer.completed_receipt(unit) {
                    Some(receipt) => receipt,
                    None => {
                        let png = fixture_png(self.next_tile as u8 + 1);
                        let receipt = writer.write_image_tile(unit, &png)?;
                        self.completed.push(CompletedUnit {
                            key: unit,
                            payload: receipt,
                        });
                        receipt
                    }
                };
                self.tiles.push(TileReceipt {
                    key,
                    payload: receipt,
                });
                self.next_tile += 1;
                self.progress.completed = self.next_tile as u32;
                if matches!(context.consume_unit(), WorkCheckpoint::Cancel(_)) {
                    return Ok(MapBakeCursorStep::Cancelled);
                }
                if self.next_tile == CountingMapProducer::TILES.len() {
                    Ok(MapBakeCursorStep::Completed(MapBakeCursorOutput::Images(
                        self.manifest()?,
                    )))
                } else {
                    Ok(MapBakeCursorStep::Yielded)
                }
            }
        }
    }

    fn progress(&self) -> MapProgressSnapshot {
        self.progress
    }

    fn take_completed_units(&mut self) -> Vec<CompletedUnit> {
        std::mem::take(&mut self.completed)
    }
}

impl MapBakeProducer for CountingMapProducer {
    fn charge(&self, _request: &BakeRequest) -> Result<WorkCharge, MapCacheError> {
        Ok(WorkCharge::new(64 * 1024, 1024 * 1024, 1024 * 1024, 0))
    }

    fn cursor(
        &self,
        request: &BakeRequest,
        partial_dir: Arc<PathBuf>,
    ) -> Result<Box<dyn MapBakeCursor>, MapCacheError> {
        match request.artifact() {
            ArtifactKind::Catalogue => {
                self.catalogues.fetch_add(1, Ordering::SeqCst);
            }
            ArtifactKind::Images => {
                self.images.fetch_add(1, Ordering::SeqCst);
            }
        }
        Ok(Box::new(CountingCursor::new(request, partial_dir)))
    }
}
