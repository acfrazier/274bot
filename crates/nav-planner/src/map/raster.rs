//! Deterministic sparse terrain raster and PNG pyramid. Base units decode only
//! a bounded mapsquare neighborhood; coarser units decode at most sixteen
//! child tiles and never construct a dense plane or client `World`.

use super::cache::{ArtifactIdentity, BakeStage, Checkpoint, CompletedUnit, PartialEntry, UnitKey};
use super::client_cache::{
    average_texture, depack_mapscene, depack_texture, load_config, load_floor_definitions,
    load_raster_locs, read_jag, CacheReader, CacheReaderCursor, Definitions, FloorDefinition,
    GroundCell, LandSquare, RasterAssets, RasterLocDefinition, MAPSCENE_COUNT,
    MAX_JAG_ARCHIVE_RETAINED_BYTES, MAX_JAG_BYTES, MAX_JAG_INDEX_TRACKED_BYTES,
    MAX_JAG_MEMBER_PEAK_BYTES, MAX_JAG_UNPACKED_BYTES, MAX_MAP_INDEX_ROWS, TEXTURE_COUNT,
};
use super::formats::{
    ColorFormat, ImageManifest, PayloadReceipt, PlaneBounds, TileReceipt, MAX_IMAGE_BYTES,
    MAX_IMAGE_TILES, MAX_JSON_BYTES, MAX_PNG_BYTES,
};
use super::identity::{BakePolicy, Digest, EncoderLibrary, ImageIdentity, IMAGE_SCHEMA};
use super::producer::{
    charge_product, charge_sum, drive_sync, make_work_charge, source_digest, ClientMapInput,
    OwnedClientMapInput, ProducerStep,
};
use super::records::{
    RecordOffset, MAX_MAP_LOC_PLACEMENTS_PER_RECORD, MAX_MAP_RECORDS, MAX_MAP_RECORD_BYTES,
};
use super::spatial::{
    TileKey, WorldBounds, MAX_LOD, TILE_GUTTER, TILE_INTERIOR, TILE_PIXELS, TILE_RGBA_BYTES,
};
use super::{MapError, Rows, Text};
use api::work::{Checkpoint as WorkCheckpoint, StepContext, WorkCharge, WorkPhase};
use client::dash3d::MapFlag;
use client::graphics::Pix8;
use client::io::JagFile;
use client::map_cache::{LocPlacement, MapIndexEntry, MAP_PLANES, MAP_SQUARE_SIZE};
use png::{BitDepth, ColorType, Compression, Decoder, Encoder, Filter};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

const FIXED_WALL_RGB: u32 = 0xeeeeee;
const FIXED_ACTIVE_WALL_RGB: u32 = 0xee0000;
const GRID_MARGIN: i32 = 5;
/// Tile receipts share one durable checkpoint after this many completed tiles.
// Four interleaved planes otherwise repeat six metadata syncs per 32 tiles.
// Keep the recovery window bounded while sharing those syncs across 64 tiles.
pub const CHECKPOINT_BATCH_TILES: usize = 64;
// TileKey::relative_path and atomic_write's `.{filename}.tmp`, at signed-i32
// coordinates and the maximum plane/LOD accepted by `TileKey::bounds`.
const MAX_TILE_RELATIVE_PATH_BYTES: usize = "terrain/p3/l26/-2147483648_-2147483648.png".len();
const MAX_TEMP_TILE_FILE_NAME_BYTES: usize = ".-2147483648_-2147483648.png.tmp".len();
const MAX_TEMP_TILE_RELATIVE_PATH_BYTES: usize =
    "terrain/p3/l26/.-2147483648_-2147483648.png.tmp".len();
const MAX_JAG_OPEN_PEAK_BYTES: u64 =
    2 * MAX_JAG_BYTES + 2 * MAX_JAG_UNPACKED_BYTES + MAX_JAG_INDEX_TRACKED_BYTES as u64;

// Native minimap masks, indexed by decoded overlay shape + 1.
const MINIMAP_SHAPE: [[u8; 16]; 13] = [
    [0; 16],
    [1; 16],
    [1, 0, 0, 0, 1, 1, 0, 0, 1, 1, 1, 0, 1, 1, 1, 1],
    [1, 1, 0, 0, 1, 1, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0],
    [0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 0, 1, 0, 0, 0, 1],
    [0, 1, 1, 1, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
    [1, 1, 1, 0, 1, 1, 1, 0, 1, 1, 1, 1, 1, 1, 1, 1],
    [1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0],
    [0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 1, 0, 0],
    [1, 1, 1, 1, 1, 1, 1, 1, 0, 1, 1, 1, 0, 0, 1, 1],
    [1, 1, 1, 1, 1, 1, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0],
    [0, 0, 0, 0, 0, 0, 1, 1, 0, 1, 1, 1, 0, 1, 1, 1],
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 1, 1, 1, 1],
];

const MINIMAP_ROTATE: [[usize; 16]; 4] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    [12, 8, 4, 0, 13, 9, 5, 1, 14, 10, 6, 2, 15, 11, 7, 3],
    [15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0],
    [3, 7, 11, 15, 2, 6, 10, 14, 1, 5, 9, 13, 0, 4, 8, 12],
];

#[derive(Debug, Clone)]
pub struct ImagePlan {
    pub identity: ImageIdentity,
    pub extent: WorldBounds,
    pub planes: Vec<PlaneBounds>,
    pub max_lod: u8,
    tiles: Vec<TileKey>,
    base_tiles: Vec<TileKey>,
}

impl ImagePlan {
    pub fn tiles(&self) -> impl ExactSizeIterator<Item = TileKey> + '_ {
        self.tiles.iter().copied()
    }

    pub fn base_tiles(&self) -> impl ExactSizeIterator<Item = TileKey> + '_ {
        self.base_tiles.iter().copied()
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PlanStats {
    pub map_squares: u32,
    pub loc_placements: u64,
    pub base_tiles: u32,
    pub pyramid_tiles: u32,
    pub max_record_bytes: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RasterStage {
    ReadingCache,
    BaseTerrain { plane: u8 },
    Downsample { plane: u8, lod: u8 },
    Publishing,
}

#[derive(Debug, Clone, Copy)]
pub struct RasterProgress {
    pub stage: RasterStage,
    pub completed_tiles: u32,
    pub total_tiles: u32,
    pub planning_squares_completed: u32,
    pub planning_squares_total: Option<u32>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct LodReceipt {
    pub plane: u8,
    pub lod: u8,
    pub tiles: u32,
    pub compressed_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct RasterMetrics {
    pub plan: PlanStats,
    pub lods: Vec<LodReceipt>,
    pub total_compressed_bytes: u64,
    pub peak_tracked_bytes: u64,
    pub elapsed: Duration,
}

#[derive(Debug, Clone)]
pub struct ImageBakeReport {
    pub manifest: ImageManifest,
    pub metrics: RasterMetrics,
}

/// Checkpoint lookup and durable tile-writing seam for owned image bakes.
pub trait ImageTileSink {
    type Error: std::error::Error + 'static;

    /// Returns the receipt already recorded for this tile, if any.
    /// Implementations MUST release any operation-local handle before returning.
    fn checkpointed(&mut self, key: TileKey) -> Option<PayloadReceipt>;

    /// `Ok(None)` means backpressure and MUST leave the tile unwritten.
    /// `Ok(Some(receipt))` means these exact PNG bytes were installed.
    /// Implementations MUST finish and release the write operation before
    /// returning, so no handle or lock can survive a cursor yield.
    fn write_tile(
        &mut self,
        key: TileKey,
        png: &[u8],
    ) -> Result<Option<PayloadReceipt>, Self::Error>;
}

#[derive(Debug, thiserror::Error)]
pub enum ImageBakeError<E: std::error::Error + 'static> {
    /// The native image bake failed.
    #[error(transparent)]
    Native(#[from] MapError),
    /// The tile sink failed.
    #[error("image tile sink failed: {0}")]
    Sink(#[source] E),
}

impl ImageBakeError<MapError> {
    fn into_map_error(self) -> MapError {
        match self {
            Self::Native(error) | Self::Sink(error) => error,
        }
    }
}

#[derive(Debug, Clone)]
pub enum ImageBakeOutcome {
    Complete(ImageBakeReport),
    Paused(RasterMetrics),
}

pub fn bake_policy() -> BakePolicy {
    BakePolicy {
        algorithm: Text::new("native-sparse-terrain-v2").expect("static policy text"),
        producer_sources: source_digest(&[
            include_bytes!("raster.rs"),
            include_bytes!("../../build.rs"),
            include_bytes!("client_cache.rs"),
            include_bytes!("records.rs"),
            include_bytes!("producer.rs"),
            include_bytes!("spatial.rs"),
            include_bytes!("../collision.rs"),
            include_bytes!("../../../../vendor/fr-client-rust/crates/client/src/map_cache.rs"),
            include_bytes!(
                "../../../../vendor/fr-client-rust/crates/client/src/config/flo_type.rs"
            ),
            include_bytes!(
                "../../../../vendor/fr-client-rust/crates/client/src/config/loc_type.rs"
            ),
            include_bytes!("../../../../vendor/fr-client-rust/crates/client/src/graphics/pix8.rs"),
            include_bytes!("../../../../vendor/fr-client-rust/crates/client/src/graphics/pix3d.rs"),
            include_bytes!(
                "../../../../vendor/fr-client-rust/crates/client/src/dash3d/map_flag.rs"
            ),
        ]),
        encoder: EncoderLibrary {
            name: Text::new("png").expect("static encoder name"),
            version: Text::new(env!("NAV_PNG_VERSION")).expect("locked encoder version"),
        },
        libraries: Rows::new(vec![
            EncoderLibrary {
                name: Text::new("crc32fast").expect("static library name"),
                version: Text::new(env!("NAV_CRC32FAST_VERSION"))
                    .expect("locked crc32fast version"),
            },
            EncoderLibrary {
                name: Text::new("fdeflate").expect("static library name"),
                version: Text::new(env!("NAV_FDEFLATE_VERSION")).expect("locked fdeflate version"),
            },
            EncoderLibrary {
                name: Text::new("flate2").expect("static library name"),
                version: Text::new(env!("NAV_FLATE2_VERSION")).expect("locked flate2 version"),
            },
            EncoderLibrary {
                name: Text::new("miniz_oxide").expect("static library name"),
                version: Text::new(env!("NAV_MINIZ_OXIDE_VERSION"))
                    .expect("locked miniz_oxide version"),
            },
        ])
        .expect("static libraries are canonical"),
        options: Text::new("rgba8;258x258;gutter=1;compression=high;filter=adaptive")
            .expect("static encoder options"),
    }
}

pub fn image_identity(input: ClientMapInput<'_>) -> Result<ImageIdentity, MapError> {
    Ok(ImageIdentity {
        revision: input.revision,
        content: input.content,
        policy: bake_policy().identity()?,
    })
}

pub fn plan_images(input: ClientMapInput<'_>) -> Result<(ImagePlan, PlanStats), MapError> {
    let mut cursor = RasterPlanCursor::new(input, None)?;
    let prepared = drive_sync(|context| cursor.step(context))?;
    Ok((prepared.plan, prepared.stats))
}

pub fn bake_images_to_partial(
    input: ClientMapInput<'_>,
    partial: &PartialEntry,
    mut keep_running: impl FnMut(RasterProgress) -> bool,
) -> Result<ImageBakeOutcome, MapError> {
    let started = Instant::now();
    let identity = image_identity(input)?;
    if partial.identity() != ArtifactIdentity::Image(identity) {
        return Err(MapError::Identity);
    }
    let reading_cache = RasterProgress {
        stage: RasterStage::ReadingCache,
        completed_tiles: 0,
        total_tiles: 0,
        planning_squares_completed: 0,
        planning_squares_total: None,
    };
    if !keep_running(reading_cache) {
        return Ok(ImageBakeOutcome::Paused(metrics_from(
            &[],
            PlanStats::default(),
            0,
            started.elapsed(),
        )));
    }
    fs::create_dir_all(partial.directory())?;
    let mut cursor =
        ImageBakeCursor::new(input, Arc::new(partial.directory().to_owned()), identity)?;
    let mut sink = AtomicImageTileSink::new(partial.directory(), |_: TileKey| None);
    let mut restored = false;
    drive_sync(|context| {
        let step = cursor
            .step_with(context, &mut sink, &mut keep_running)
            .map_err(ImageBakeError::into_map_error)?;
        if cursor.plan().is_some() && !restored {
            let total_tiles = cursor
                .planned_units()
                .ok_or(MapError::Limit("tile count"))?;
            let checkpoint_path = partial.directory().join("checkpoint.json");
            let completed = if checkpoint_path.exists() {
                let checkpoint = partial.load_checkpoint()?;
                if checkpoint.planned_units != total_tiles {
                    return Err(MapError::Invalid("checkpoint plan count"));
                }
                checkpoint.completed.as_slice().to_vec()
            } else {
                write_checkpoint(
                    partial.directory(),
                    identity,
                    BakeStage::BaseTerrain,
                    total_tiles,
                    &[],
                )?;
                Vec::new()
            };
            cursor.resume_completed(completed)?;
            cursor.enable_checkpointing();
            restored = true;
        }
        match step {
            ProducerStep::Yielded if cursor.was_paused_by_callback() => Ok(
                ProducerStep::Completed(ImageBakeOutcome::Paused(metrics_from(
                    &cursor.completed,
                    cursor.plan_stats,
                    cursor.tracker.peak,
                    cursor.started.elapsed(),
                ))),
            ),
            ProducerStep::Yielded => Ok(ProducerStep::Yielded),
            ProducerStep::Completed(report) => {
                Ok(ProducerStep::Completed(ImageBakeOutcome::Complete(report)))
            }
            ProducerStep::Cancelled => Ok(ProducerStep::Cancelled),
        }
    })
}

pub fn bake_images_into(
    input: ClientMapInput<'_>,
    tile_dir: &Path,
    checkpointed: impl FnMut(TileKey) -> Option<PayloadReceipt>,
    mut keep_running: impl FnMut(RasterProgress) -> bool,
) -> Result<ImageBakeOutcome, MapError> {
    let started = Instant::now();
    let identity = image_identity(input)?;
    let reading_cache = RasterProgress {
        stage: RasterStage::ReadingCache,
        completed_tiles: 0,
        total_tiles: 0,
        planning_squares_completed: 0,
        planning_squares_total: None,
    };
    if !keep_running(reading_cache) {
        return Ok(ImageBakeOutcome::Paused(metrics_from(
            &[],
            PlanStats::default(),
            0,
            started.elapsed(),
        )));
    }
    fs::create_dir_all(tile_dir)?;
    let mut sink = AtomicImageTileSink::new(tile_dir, checkpointed);
    let mut cursor = ImageBakeCursor::new(input, Arc::new(tile_dir.to_owned()), identity)?;
    drive_sync(|context| {
        match cursor
            .step_with(context, &mut sink, &mut keep_running)
            .map_err(ImageBakeError::into_map_error)?
        {
            ProducerStep::Yielded if cursor.was_paused_by_callback() => Ok(
                ProducerStep::Completed(ImageBakeOutcome::Paused(metrics_from(
                    &cursor.completed,
                    cursor.plan_stats,
                    cursor.tracker.peak,
                    cursor.started.elapsed(),
                ))),
            ),
            ProducerStep::Yielded => Ok(ProducerStep::Yielded),
            ProducerStep::Completed(report) => {
                Ok(ProducerStep::Completed(ImageBakeOutcome::Complete(report)))
            }
            ProducerStep::Cancelled => Ok(ProducerStep::Cancelled),
        }
    })
}

/// Receipt for a tile file on disk. The receipt comes from the file itself,
/// so the header check alone would accept a file damaged after its IHDR;
/// the tile is fully decoded (chunk CRCs, zlib stream, 258×258 RGBA8).
fn completed_from_disk(
    directory: &Path,
    key: TileKey,
    compressed: &mut Vec<u8>,
    pixels: &mut Vec<u8>,
) -> Result<CompletedUnit, MapError> {
    read_bounded(
        &directory.join(key.relative_path()?),
        MAX_PNG_BYTES as usize,
        compressed,
    )?;
    let payload = PayloadReceipt {
        bytes: compressed.len() as u32,
        sha256: Digest::of(compressed),
    };
    TileReceipt { key, payload }.verify_png(compressed)?;
    decode_png_into(compressed, pixels)?;
    Ok(CompletedUnit {
        key: UnitKey::Terrain { tile: key },
        payload,
    })
}

fn effective_plane(raw_plane: u8, link_below: bool) -> Option<u8> {
    u8::try_from(crate::collision::game_plane(
        i32::from(raw_plane),
        link_below,
    )?)
    .ok()
}

fn minimap_plane(effective_plane: u8, flags: u8) -> Option<u8> {
    if i32::from(flags) & MapFlag::VIS_BELOW != 0 {
        effective_plane.checked_sub(1)
    } else if i32::from(flags) & MapFlag::FORCE_HIGH_DETAIL != 0 {
        None
    } else {
        Some(effective_plane)
    }
}

fn validate_image_tile_count(count: usize) -> Result<(), MapError> {
    if count > MAX_IMAGE_TILES {
        Err(MapError::Limit("image tile count"))
    } else {
        Ok(())
    }
}

struct RasterContext {
    definitions: Definitions,
    assets: RasterAssets,
    resources_bytes: u64,
    reader: CacheReader,
}

impl RasterContext {
    fn new(definitions: Definitions, assets: RasterAssets, reader: CacheReader) -> Self {
        // Definitions and assets are immutable once preparation finishes.
        let resources_bytes = raster_resources_bytes(&definitions, &assets);
        Self {
            definitions,
            assets,
            resources_bytes,
            reader,
        }
    }

    fn tracked_bytes(&self) -> u64 {
        self.resources_bytes + self.reader.tracked_bytes() as u64
    }
}

fn raster_resources_bytes(definitions: &Definitions, assets: &RasterAssets) -> u64 {
    let definitions = definitions.locs.capacity() * std::mem::size_of::<RasterLocDefinition>()
        + definitions.floors.capacity() * std::mem::size_of::<FloorDefinition>();
    let sprites = assets.mapscenes.capacity() * std::mem::size_of::<Option<Pix8>>()
        + assets
            .mapscenes
            .iter()
            .flatten()
            .map(|sprite| sprite.data.capacity() + sprite.bpal.capacity() * 4)
            .sum::<usize>();
    (definitions + sprites + std::mem::size_of_val(&assets.texture_colours)) as u64
}

enum RasterPlanPhase {
    Identity,
    OpenLocConfig,
    DecodeLocs,
    OpenFloorConfig,
    DecodeFloors,
    OpenMapsceneArchive { id: usize },
    DecodeMapscene { id: usize },
    OpenTextureArchive { id: usize },
    DecodeTexture { id: usize },
    ReaderInit,
    Reader,
    Extent,
    Land,
    TerrainCell,
    LocRecord,
    Placement,
    FinalizeSquare { plane: u8 },
    SeedTiles,
    Pyramid,
    PlaneBounds { plane: u8 },
    BuildResult,
    Complete,
}

struct RasterPrepared {
    context: RasterContext,
    plan: ImagePlan,
    stats: PlanStats,
}

enum RasterPlanAction {
    Transition,
    Unit,
    Completed,
}

/// Incremental preparation and sparse-plan construction shared by all raster
/// entry points. A step never retains a live file handle.
struct RasterPlanCursor {
    input: OwnedClientMapInput,
    expected_identity: Option<ImageIdentity>,
    identity: Option<ImageIdentity>,
    phase: RasterPlanPhase,
    config: Option<JagFile>,
    media: Option<JagFile>,
    textures: Option<JagFile>,
    definitions: Definitions,
    assets: RasterAssets,
    reader_cursor: Option<CacheReaderCursor>,
    context: Option<RasterContext>,
    extent_index: usize,
    extent: Option<WorldBounds>,
    entry_index: usize,
    land: Option<LandSquare>,
    terrain_cell_index: usize,
    present: [bool; 4],
    placements: Vec<LocPlacement>,
    placement_index: usize,
    base_tiles: BTreeSet<TileKey>,
    base_iter: Option<std::collections::btree_set::IntoIter<TileKey>>,
    base_output: Vec<TileKey>,
    tiles: BTreeSet<TileKey>,
    level: Vec<TileKey>,
    level_index: usize,
    next_level: BTreeSet<TileKey>,
    plane_counts: [u32; 4],
    next_plane_counts: [u32; 4],
    plane_bounds: [Option<(i32, i32, i32, i32)>; 4],
    pyramid_lod: u8,
    stats: PlanStats,
    peak_tracked_bytes: u64,
    result: Option<RasterPrepared>,
    completed: bool,
}

impl RasterPlanCursor {
    fn new(
        input: ClientMapInput<'_>,
        expected_identity: Option<ImageIdentity>,
    ) -> Result<Self, MapError> {
        Ok(Self {
            input: OwnedClientMapInput::copy(input)?,
            expected_identity,
            identity: None,
            phase: RasterPlanPhase::Identity,
            config: None,
            media: None,
            textures: None,
            definitions: Definitions {
                locs: Vec::new(),
                floors: Vec::new(),
            },
            assets: RasterAssets {
                mapscenes: Vec::with_capacity(MAPSCENE_COUNT),
                texture_colours: [0; TEXTURE_COUNT],
            },
            reader_cursor: None,
            context: None,
            extent_index: 0,
            extent: None,
            entry_index: 0,
            land: None,
            terrain_cell_index: 0,
            present: [false; 4],
            placements: Vec::new(),
            placement_index: 0,
            base_tiles: BTreeSet::new(),
            base_iter: None,
            base_output: Vec::new(),
            tiles: BTreeSet::new(),
            level: Vec::new(),
            level_index: 0,
            next_level: BTreeSet::new(),
            plane_counts: [0; 4],
            next_plane_counts: [0; 4],
            plane_bounds: [None; 4],
            pyramid_lod: 0,
            stats: PlanStats::default(),
            peak_tracked_bytes: 0,
            result: None,
            completed: false,
        })
    }

    fn step<P: WorkPhase>(
        &mut self,
        context: &mut StepContext<'_, P>,
    ) -> Result<ProducerStep<RasterPrepared>, MapError> {
        if self.completed {
            return Err(MapError::Invalid("raster plan cursor complete"));
        }
        match context.checkpoint() {
            WorkCheckpoint::Continue => {}
            WorkCheckpoint::Yield => return Ok(ProducerStep::Yielded),
            WorkCheckpoint::Cancel(_) => return Ok(ProducerStep::Cancelled),
        }
        loop {
            match self.advance()? {
                // Check each edge once, including every completed unit.
                // Deadline overrun remains bounded by the same single unit.
                RasterPlanAction::Transition => match context.checkpoint() {
                    WorkCheckpoint::Continue => {}
                    WorkCheckpoint::Yield => return Ok(ProducerStep::Yielded),
                    WorkCheckpoint::Cancel(_) => return Ok(ProducerStep::Cancelled),
                },
                RasterPlanAction::Unit => {
                    self.observe_memory();
                    match context.consume_unit() {
                        WorkCheckpoint::Continue => {}
                        WorkCheckpoint::Yield => return Ok(ProducerStep::Yielded),
                        WorkCheckpoint::Cancel(_) => return Ok(ProducerStep::Cancelled),
                    }
                }
                RasterPlanAction::Completed => match context.checkpoint() {
                    WorkCheckpoint::Continue => {
                        let result = self
                            .result
                            .take()
                            .ok_or(MapError::Invalid("missing completed raster plan"))?;
                        self.completed = true;
                        return Ok(ProducerStep::Completed(result));
                    }
                    WorkCheckpoint::Yield => return Ok(ProducerStep::Yielded),
                    WorkCheckpoint::Cancel(_) => return Ok(ProducerStep::Cancelled),
                },
            }
        }
    }

    fn planning_squares_progress(&self) -> Option<(u32, u32)> {
        let total = self.context.as_ref()?.reader.entries.len();
        Some((
            u32::try_from(self.entry_index.min(total)).ok()?,
            u32::try_from(total).ok()?,
        ))
    }

    fn advance(&mut self) -> Result<RasterPlanAction, MapError> {
        let phase = std::mem::replace(&mut self.phase, RasterPlanPhase::Complete);
        match phase {
            RasterPlanPhase::Identity => {
                let identity = image_identity(self.input.view())?;
                if self
                    .expected_identity
                    .is_some_and(|expected| expected != identity)
                {
                    return Err(MapError::Identity);
                }
                self.identity = Some(identity);
                self.phase = RasterPlanPhase::OpenLocConfig;
                Ok(RasterPlanAction::Unit)
            }
            RasterPlanPhase::OpenLocConfig => {
                self.config = Some(load_config(self.input.view())?);
                self.phase = RasterPlanPhase::DecodeLocs;
                Ok(RasterPlanAction::Unit)
            }
            RasterPlanPhase::DecodeLocs => {
                let jag = self
                    .config
                    .as_ref()
                    .ok_or(MapError::Invalid("missing location config JAG"))?;
                self.definitions.locs = load_raster_locs(jag)?;
                self.config = None;
                self.phase = RasterPlanPhase::OpenFloorConfig;
                Ok(RasterPlanAction::Unit)
            }
            RasterPlanPhase::OpenFloorConfig => {
                self.config = Some(load_config(self.input.view())?);
                self.phase = RasterPlanPhase::DecodeFloors;
                Ok(RasterPlanAction::Unit)
            }
            RasterPlanPhase::DecodeFloors => {
                let jag = self
                    .config
                    .as_ref()
                    .ok_or(MapError::Invalid("missing floor config JAG"))?;
                self.definitions.floors = load_floor_definitions(jag)?;
                self.config = None;
                self.phase = RasterPlanPhase::OpenMapsceneArchive { id: 0 };
                Ok(RasterPlanAction::Unit)
            }
            RasterPlanPhase::OpenMapsceneArchive { id } => {
                self.media = Some(read_jag(
                    &self.input.view().jag_dir.join("media"),
                    "media JAG",
                )?);
                self.phase = RasterPlanPhase::DecodeMapscene { id };
                Ok(RasterPlanAction::Unit)
            }
            RasterPlanPhase::DecodeMapscene { id } => {
                let media = self
                    .media
                    .as_ref()
                    .ok_or(MapError::Invalid("missing media JAG"))?;
                self.assets.mapscenes.push(depack_mapscene(media, id)?);
                self.media = None;
                if id + 1 == MAPSCENE_COUNT {
                    self.phase = RasterPlanPhase::OpenTextureArchive { id: 0 };
                } else {
                    self.phase = RasterPlanPhase::OpenMapsceneArchive { id: id + 1 };
                }
                Ok(RasterPlanAction::Unit)
            }
            RasterPlanPhase::OpenTextureArchive { id } => {
                self.textures = Some(read_jag(
                    &self.input.view().jag_dir.join("textures"),
                    "textures JAG",
                )?);
                self.phase = RasterPlanPhase::DecodeTexture { id };
                Ok(RasterPlanAction::Unit)
            }
            RasterPlanPhase::DecodeTexture { id } => {
                let textures = self
                    .textures
                    .as_ref()
                    .ok_or(MapError::Invalid("missing textures JAG"))?;
                if let Some(texture) = depack_texture(textures, id)? {
                    self.assets.texture_colours[id] = average_texture(&texture);
                }
                if id + 1 == TEXTURE_COUNT {
                    self.textures = None;
                    self.phase = RasterPlanPhase::ReaderInit;
                } else {
                    self.phase = RasterPlanPhase::OpenTextureArchive { id: id + 1 };
                }
                Ok(RasterPlanAction::Unit)
            }
            RasterPlanPhase::ReaderInit => {
                self.reader_cursor = Some(CacheReaderCursor::new(self.input.view()));
                self.phase = RasterPlanPhase::Reader;
                Ok(RasterPlanAction::Transition)
            }
            RasterPlanPhase::Reader => {
                let reader_cursor = self
                    .reader_cursor
                    .as_mut()
                    .ok_or(MapError::Invalid("missing cache reader cursor"))?;
                if let Some(reader) = reader_cursor.advance()? {
                    self.stats.map_squares = reader.entries.len() as u32;
                    let definitions = std::mem::replace(
                        &mut self.definitions,
                        Definitions {
                            locs: Vec::new(),
                            floors: Vec::new(),
                        },
                    );
                    let assets = std::mem::replace(
                        &mut self.assets,
                        RasterAssets {
                            mapscenes: Vec::new(),
                            texture_colours: [0; TEXTURE_COUNT],
                        },
                    );
                    self.context = Some(RasterContext::new(definitions, assets, reader));
                    self.reader_cursor = None;
                    self.phase = RasterPlanPhase::Extent;
                } else {
                    self.phase = RasterPlanPhase::Reader;
                }
                Ok(RasterPlanAction::Unit)
            }
            RasterPlanPhase::Extent => {
                let entries = &self
                    .context
                    .as_ref()
                    .ok_or(MapError::Invalid("missing raster context"))?
                    .reader
                    .entries;
                let Some(entry) = entries.get(self.extent_index).copied() else {
                    if self.extent.is_none() {
                        return Err(MapError::Invalid("empty map index"));
                    }
                    self.phase = RasterPlanPhase::Land;
                    return Ok(RasterPlanAction::Transition);
                };
                let x = i32::from(entry.square_x);
                let z = i32::from(entry.square_z);
                self.extent = Some(match self.extent {
                    Some(extent) => WorldBounds {
                        west: extent.west.min(x * 64),
                        south: extent.south.min(z * 64),
                        east: extent.east.max((x + 1) * 64),
                        north: extent.north.max((z + 1) * 64),
                    },
                    None => WorldBounds {
                        west: x * 64,
                        south: z * 64,
                        east: (x + 1) * 64,
                        north: (z + 1) * 64,
                    },
                });
                self.extent_index += 1;
                self.phase = RasterPlanPhase::Extent;
                Ok(RasterPlanAction::Unit)
            }
            RasterPlanPhase::Land => {
                let context = self
                    .context
                    .as_mut()
                    .ok_or(MapError::Invalid("missing raster context"))?;
                let Some(entry) = context.reader.entries.get(self.entry_index).copied() else {
                    if self.base_tiles.is_empty() {
                        return Err(MapError::Invalid("empty client imagery"));
                    }
                    self.base_iter = Some(std::mem::take(&mut self.base_tiles).into_iter());
                    self.phase = RasterPlanPhase::SeedTiles;
                    return Ok(RasterPlanAction::Transition);
                };
                self.land = Some(context.reader.read_land(entry)?);
                self.present = [false; 4];
                self.terrain_cell_index = 0;
                self.phase = RasterPlanPhase::TerrainCell;
                Ok(RasterPlanAction::Unit)
            }
            RasterPlanPhase::TerrainCell => {
                let Some(land) = self.land.as_ref() else {
                    return Err(MapError::Invalid("missing raster land square"));
                };
                let cell_count = 4 * usize::from(MAP_SQUARE_SIZE) * usize::from(MAP_SQUARE_SIZE);
                if self.terrain_cell_index == cell_count {
                    self.phase = RasterPlanPhase::LocRecord;
                    return Ok(RasterPlanAction::Transition);
                }
                let index = self.terrain_cell_index;
                let raw_plane = (index % 4) as u8;
                let z = ((index / 4) % usize::from(MAP_SQUARE_SIZE)) as u8;
                let x = (index / (4 * usize::from(MAP_SQUARE_SIZE))) as u8;
                self.terrain_cell_index += 1;
                let link_below = land.link_below(x, z);
                let cell = land.cell(raw_plane, x, z);
                if cell.underlay != 0 || cell.overlay != 0 {
                    if let Some(effective_plane) = effective_plane(raw_plane, link_below) {
                        let flags = land.cell(effective_plane, x, z).flags;
                        if let Some(plane) = minimap_plane(effective_plane, flags) {
                            self.present[usize::from(plane)] = true;
                        }
                    }
                }
                self.phase = RasterPlanPhase::TerrainCell;
                Ok(RasterPlanAction::Unit)
            }
            RasterPlanPhase::LocRecord => {
                let entry = self.current_entry()?;
                self.placements = self
                    .context
                    .as_mut()
                    .ok_or(MapError::Invalid("missing raster context"))?
                    .reader
                    .read_locs(entry)?;
                self.placement_index = 0;
                self.phase = RasterPlanPhase::Placement;
                Ok(RasterPlanAction::Unit)
            }
            RasterPlanPhase::Placement => {
                let Some(placement) = self.placements.get(self.placement_index).copied() else {
                    self.phase = RasterPlanPhase::FinalizeSquare { plane: 0 };
                    return Ok(RasterPlanAction::Transition);
                };
                self.placement_index += 1;
                self.stats.loc_placements += 1;
                self.phase = RasterPlanPhase::Placement;
                let entry = self.current_entry()?;
                let land = self
                    .land
                    .as_ref()
                    .ok_or(MapError::Invalid("missing raster land square"))?;
                let definition = self
                    .context
                    .as_ref()
                    .ok_or(MapError::Invalid("missing raster context"))?
                    .definitions
                    .locs
                    .get(placement.id as usize)
                    .ok_or(MapError::Invalid("location definition id"))?;
                let link_below = land.link_below(placement.x, placement.z);
                let Some(effective_plane) = effective_plane(placement.plane, link_below) else {
                    return Ok(RasterPlanAction::Unit);
                };
                let flags = land.cell(effective_plane, placement.x, placement.z).flags;
                let Some(plane) = minimap_plane(effective_plane, flags) else {
                    return Ok(RasterPlanAction::Unit);
                };
                if (definition.mapscene.is_some() && !matches!(placement.shape, 4..=8))
                    || draws_native_detail(placement.shape)
                {
                    self.present[plane as usize] = true;
                }
                let extent = self
                    .extent
                    .ok_or(MapError::Invalid("missing raster extent"))?;
                let assets = &self
                    .context
                    .as_ref()
                    .ok_or(MapError::Invalid("missing raster context"))?
                    .assets;
                mark_mapscene_tiles(
                    &mut self.base_tiles,
                    extent,
                    entry,
                    placement,
                    definition,
                    assets,
                    plane,
                )?;
                validate_image_tile_count(self.base_tiles.len())?;
                Ok(RasterPlanAction::Unit)
            }
            RasterPlanPhase::FinalizeSquare { plane } => {
                if plane >= 4 {
                    self.entry_index += 1;
                    self.land = None;
                    self.placements.clear();
                    self.placement_index = 0;
                    self.phase = RasterPlanPhase::Land;
                    return Ok(RasterPlanAction::Transition);
                }
                if self.present[usize::from(plane)] {
                    let entry = self.current_entry()?;
                    let key = TileKey {
                        plane,
                        lod: 0,
                        x: i32::from(entry.square_x),
                        z: i32::from(entry.square_z),
                    };
                    self.base_tiles.insert(key);
                    validate_image_tile_count(self.base_tiles.len())?;
                }
                self.phase = RasterPlanPhase::FinalizeSquare { plane: plane + 1 };
                Ok(RasterPlanAction::Unit)
            }
            RasterPlanPhase::SeedTiles => {
                let tile = self.base_iter.as_mut().and_then(Iterator::next);
                if let Some(tile) = tile {
                    self.base_output.push(tile);
                    self.tiles.insert(tile);
                    self.level.push(tile);
                    let plane = usize::from(tile.plane);
                    self.plane_counts[plane] += 1;
                    let x = tile.x;
                    let z = tile.z;
                    self.plane_bounds[plane] = Some(match self.plane_bounds[plane] {
                        Some((west, south, east, north)) => {
                            (west.min(x), south.min(z), east.max(x + 1), north.max(z + 1))
                        }
                        None => (x, z, x + 1, z + 1),
                    });
                    self.phase = RasterPlanPhase::SeedTiles;
                    Ok(RasterPlanAction::Unit)
                } else {
                    self.base_iter = None;
                    if only_one_per_present_plane(&self.plane_counts) {
                        self.phase = RasterPlanPhase::PlaneBounds { plane: 0 };
                    } else {
                        self.phase = RasterPlanPhase::Pyramid;
                    }
                    Ok(RasterPlanAction::Transition)
                }
            }
            RasterPlanPhase::Pyramid => {
                if self.level_index < self.level.len() {
                    let tile = self.level[self.level_index];
                    self.level_index += 1;
                    let parent = TileKey {
                        plane: tile.plane,
                        lod: self.pyramid_lod + 1,
                        x: tile.x.div_euclid(2),
                        z: tile.z.div_euclid(2),
                    };
                    if self.next_level.insert(parent) {
                        self.next_plane_counts[usize::from(parent.plane)] += 1;
                        self.tiles.insert(parent);
                        validate_image_tile_count(self.tiles.len())?;
                    }
                    self.phase = RasterPlanPhase::Pyramid;
                    return Ok(RasterPlanAction::Unit);
                }
                let next_lod = self
                    .pyramid_lod
                    .checked_add(1)
                    .ok_or(MapError::Limit("pyramid LOD"))?;
                if next_lod > MAX_LOD {
                    return Err(MapError::Limit("pyramid LOD"));
                }
                self.pyramid_lod = next_lod;
                self.plane_counts = self.next_plane_counts;
                if only_one_per_present_plane(&self.plane_counts) {
                    self.phase = RasterPlanPhase::PlaneBounds { plane: 0 };
                } else {
                    self.level = std::mem::take(&mut self.next_level).into_iter().collect();
                    self.level_index = 0;
                    self.next_plane_counts = [0; 4];
                    self.phase = RasterPlanPhase::Pyramid;
                }
                Ok(RasterPlanAction::Transition)
            }
            RasterPlanPhase::PlaneBounds { plane } => {
                if plane >= 4 {
                    self.phase = RasterPlanPhase::BuildResult;
                    return Ok(RasterPlanAction::Transition);
                }
                self.phase = RasterPlanPhase::PlaneBounds { plane: plane + 1 };
                Ok(RasterPlanAction::Unit)
            }
            RasterPlanPhase::BuildResult => {
                let identity = self
                    .identity
                    .ok_or(MapError::Invalid("missing image identity"))?;
                let planes = self
                    .plane_bounds
                    .iter()
                    .enumerate()
                    .filter_map(|(plane, bounds)| {
                        bounds.map(|(west, south, east, north)| PlaneBounds {
                            plane: plane as u8,
                            bounds: WorldBounds {
                                west: west * 64,
                                south: south * 64,
                                east: east * 64,
                                north: north * 64,
                            },
                        })
                    })
                    .collect();
                self.stats.base_tiles = self.base_output.len() as u32;
                self.stats.pyramid_tiles = self.tiles.len() as u32;
                self.stats.max_record_bytes = self
                    .context
                    .as_ref()
                    .ok_or(MapError::Invalid("missing raster context"))?
                    .reader
                    .max_buffer_bytes();
                let context = self
                    .context
                    .take()
                    .ok_or(MapError::Invalid("missing raster context"))?;
                let plan = ImagePlan {
                    identity,
                    extent: self
                        .extent
                        .ok_or(MapError::Invalid("missing raster extent"))?,
                    planes,
                    max_lod: self.pyramid_lod,
                    tiles: std::mem::take(&mut self.tiles).into_iter().collect(),
                    base_tiles: std::mem::take(&mut self.base_output),
                };
                self.result = Some(RasterPrepared {
                    context,
                    plan,
                    stats: self.stats,
                });
                self.phase = RasterPlanPhase::Complete;
                Ok(RasterPlanAction::Unit)
            }
            RasterPlanPhase::Complete => Ok(RasterPlanAction::Completed),
        }
    }

    fn current_entry(&self) -> Result<MapIndexEntry, MapError> {
        self.context
            .as_ref()
            .and_then(|context| context.reader.entries.get(self.entry_index).copied())
            .ok_or(MapError::Invalid("missing raster map square"))
    }
}

fn only_one_per_present_plane(counts: &[u32; 4]) -> bool {
    counts.iter().all(|count| *count == 0 || *count == 1)
}

impl RasterPlanCursor {
    fn observe_memory(&mut self) {
        let mut bytes = raster_resources_bytes(&self.definitions, &self.assets);
        if let Some(context) = &self.context {
            bytes += context.tracked_bytes();
        }
        if let Some(reader) = &self.reader_cursor {
            bytes += reader.tracked_bytes() as u64;
        }
        if self.config.is_some() || self.media.is_some() || self.textures.is_some() {
            bytes += MAX_JAG_ARCHIVE_RETAINED_BYTES;
        }
        if matches!(
            &self.phase,
            RasterPlanPhase::OpenLocConfig
                | RasterPlanPhase::OpenFloorConfig
                | RasterPlanPhase::OpenMapsceneArchive { .. }
                | RasterPlanPhase::OpenTextureArchive { .. }
                | RasterPlanPhase::Reader
        ) {
            bytes += MAX_JAG_OPEN_PEAK_BYTES;
        } else if matches!(
            &self.phase,
            RasterPlanPhase::DecodeLocs
                | RasterPlanPhase::DecodeFloors
                | RasterPlanPhase::DecodeMapscene { .. }
                | RasterPlanPhase::DecodeTexture { .. }
        ) {
            bytes += MAX_JAG_MEMBER_PEAK_BYTES;
        }
        bytes += self.placements.capacity() as u64 * std::mem::size_of::<LocPlacement>() as u64;
        bytes += self
            .land
            .as_ref()
            .map_or(0, |land| land.cells_capacity_bytes() as u64);
        bytes += (self.base_tiles.len() + self.tiles.len() + self.next_level.len()) as u64
            * (std::mem::size_of::<TileKey>() as u64 + 3 * std::mem::size_of::<usize>() as u64);
        bytes += (self.base_output.capacity() + self.level.capacity()) as u64
            * std::mem::size_of::<TileKey>() as u64;
        if let Some(result) = &self.result {
            bytes += result.context.tracked_bytes();
            bytes += (result.plan.tiles.capacity() + result.plan.base_tiles.capacity()) as u64
                * std::mem::size_of::<TileKey>() as u64;
            bytes +=
                result.plan.planes.capacity() as u64 * std::mem::size_of::<PlaneBounds>() as u64;
        }
        self.peak_tracked_bytes = self.peak_tracked_bytes.max(bytes);
    }
}

enum RasterBakePhase {
    Plan,
    BuildGroups,
    Adopt,
    StartBaseGroup,
    LoadNeighborLand,
    LoadNeighborLocs,
    RasterizeBase,
    EncodeBase,
    WriteBase,
    DownsampleCheckpoint,
    DownsampleScan,
    LoadParentChild,
    RasterizeParentRow,
    EncodeParent,
    WriteParent,
    ManifestReceipts,
    ManifestBuild,
    ManifestWrite,
    PauseCheckpoint,
    Complete,
}

enum RasterBakeAction {
    Transition,
    Unit,
    Backpressure,
    Paused,
    Completed,
}

/// Owned resumable image producer. It performs planning once, yields with the
/// complete plan before inspecting/adopting tiles, then publishes one tile at
/// a time. No operation file handle or borrowed input survives a yielded step.
pub struct ImageBakeCursor {
    planner: Option<RasterPlanCursor>,
    expected_identity: ImageIdentity,
    directory: Arc<PathBuf>,
    phase: RasterBakePhase,
    context: Option<RasterContext>,
    plan: Option<ImagePlan>,
    plan_stats: PlanStats,
    progress: RasterProgress,
    started: Instant,
    groups_builder: BTreeMap<(i32, i32), Vec<u8>>,
    groups: Vec<(i32, i32, Vec<u8>)>,
    base_key_index: usize,
    group_index: usize,
    base_plane_index: usize,
    neighbor_index: usize,
    pending_neighbor: Option<(i32, i32, MapIndexEntry, LandSquare)>,
    neighborhood: Option<Neighborhood>,
    adoption_index: usize,
    downsample_index: usize,
    completed: Vec<CompletedUnit>,
    completed_keys: BTreeSet<TileKey>,
    completed_bytes: u64,
    reported_completed: Vec<CompletedUnit>,
    current_tile: Option<TileKey>,
    rgba: Vec<u8>,
    png: Vec<u8>,
    compressed: Vec<u8>,
    pixels: Vec<u8>,
    parent_children: [Option<Vec<u8>>; 16],
    parent_child_index: usize,
    parent_row: usize,
    receipts: Vec<TileReceipt>,
    manifest: Option<ImageManifest>,
    result: Option<ImageBakeReport>,
    tracker: MemoryTracker,
    checkpoint_enabled: bool,
    adopt_existing: bool,
    checkpoint_stage: Option<BakeStage>,
    paused_by_callback: bool,
    completed_cursor: bool,
}

impl ImageBakeCursor {
    /// Pure upper bound for the cursor charge. It reads only typed limits and
    /// the paths that the cursor will own; it does not open or decode inputs.
    pub fn work_charge(
        input: ClientMapInput<'_>,
        partial_dir: &Arc<PathBuf>,
    ) -> Result<WorkCharge, MapError> {
        if partial_dir.as_os_str().is_empty() {
            return Err(MapError::Path);
        }
        let max_definitions = u16::MAX as usize;
        let max_placement_capacity = charge_product(MAX_MAP_LOC_PLACEMENTS_PER_RECORD, 2)?;
        let placement_bytes =
            charge_product(max_placement_capacity, std::mem::size_of::<LocPlacement>())?;
        let land_bytes = charge_product(4 * 64 * 64, std::mem::size_of::<GroundCell>())?;
        let square_bytes = charge_sum([land_bytes, placement_bytes])?;
        let neighborhood_bytes = charge_product(9, square_bytes)?;
        let loc_grid_bytes = charge_product(
            (66 + 2 * GRID_MARGIN) as usize * (66 + 2 * GRID_MARGIN) as usize,
            std::mem::size_of::<Option<MinimapCell>>(),
        )?;
        let partial_owner_bytes = charge_sum([
            partial_dir.capacity(),
            std::mem::size_of::<PathBuf>(),
            charge_product(2, std::mem::size_of::<usize>())?,
        ])?;
        let input_paths = charge_sum([
            charge_product(2, input.jag_dir.as_os_str().len())?,
            charge_product(2, input.snapshot_dir.as_os_str().len())?,
            "maps.bin".len(),
            partial_owner_bytes,
        ])?;
        let scene_bytes = charge_sum([
            MAX_JAG_UNPACKED_BYTES as usize,
            charge_product(MAPSCENE_COUNT, 255 * std::mem::size_of::<i32>())?,
            charge_product(MAPSCENE_COUNT, std::mem::size_of::<Option<Pix8>>())?,
            charge_product(TEXTURE_COUNT, std::mem::size_of::<u32>())?,
        ])?;
        let cache_bytes = charge_sum([
            charge_product(MAX_MAP_INDEX_ROWS, std::mem::size_of::<MapIndexEntry>())?,
            charge_product(MAX_MAP_RECORDS, std::mem::size_of::<RecordOffset>())?,
            MAX_MAP_RECORD_BYTES,
            MAX_MAP_INDEX_ROWS * client::map_cache::MAP_INDEX_ROW_BYTES,
        ])?;
        let plan_node_bytes = std::mem::size_of::<TileKey>() + 3 * std::mem::size_of::<usize>();
        let plan_bytes = charge_sum([
            charge_product(2 * MAX_IMAGE_TILES, std::mem::size_of::<TileKey>())?,
            charge_product(3 * MAX_IMAGE_TILES, plan_node_bytes)?,
            charge_product(2 * MAX_IMAGE_TILES, std::mem::size_of::<TileKey>())?,
            charge_product(MAX_IMAGE_TILES, std::mem::size_of::<(i32, i32, Vec<u8>)>())?,
            charge_product(
                2 * MAX_IMAGE_TILES * usize::from(MAP_PLANES),
                std::mem::size_of::<u8>(),
            )?,
            charge_product(
                3 * MAX_IMAGE_TILES,
                2 * std::mem::size_of::<CompletedUnit>(),
            )?,
            charge_product(MAX_IMAGE_TILES, std::mem::size_of::<TileReceipt>())?,
        ])?;
        let neighborhood_map_nodes = charge_product(
            9,
            std::mem::size_of::<((i32, i32), SquareData)>() + 3 * std::mem::size_of::<usize>(),
        )?;
        let bake_buffers = charge_sum([
            charge_product(18, TILE_RGBA_BYTES)?,
            charge_product(2, MAX_PNG_BYTES as usize)?,
        ])?;
        let input_bytes = charge_sum([
            std::mem::size_of::<Self>(),
            input_paths,
            scene_bytes,
            charge_product(max_definitions, std::mem::size_of::<RasterLocDefinition>())?,
            charge_product(max_definitions, std::mem::size_of::<FloorDefinition>())?,
            MAX_JAG_ARCHIVE_RETAINED_BYTES as usize,
            cache_bytes,
            plan_bytes,
            neighborhood_bytes,
            neighborhood_map_nodes,
            square_bytes,
            bake_buffers,
        ])?;
        let jag_open_scratch = (MAX_JAG_OPEN_PEAK_BYTES - MAX_JAG_ARCHIVE_RETAINED_BYTES) as usize;
        let jag_member_scratch =
            (MAX_JAG_MEMBER_PEAK_BYTES - MAX_JAG_ARCHIVE_RETAINED_BYTES) as usize;
        let relative_path_capacity = charge_product(2, MAX_TILE_RELATIVE_PATH_BYTES)?;
        let target_path_capacity = charge_product(
            2,
            charge_sum([partial_dir.capacity(), MAX_TILE_RELATIVE_PATH_BYTES])?,
        )?;
        let temporary_filename_capacity = charge_product(2, MAX_TEMP_TILE_FILE_NAME_BYTES)?;
        let temporary_path_capacity = charge_product(
            2,
            charge_sum([partial_dir.capacity(), MAX_TEMP_TILE_RELATIVE_PATH_BYTES])?,
        )?;
        let path_values = charge_product(2, std::mem::size_of::<PathBuf>())?;
        let path_build_scratch =
            charge_sum([relative_path_capacity, target_path_capacity, path_values])?;
        let atomic_write_path_scratch = charge_sum([
            target_path_capacity,
            temporary_filename_capacity,
            temporary_path_capacity,
            path_values,
        ])?;
        let temporary_path_bytes = path_build_scratch.max(atomic_write_path_scratch);
        let checkpoint_scratch = charge_sum([
            charge_product(MAX_IMAGE_TILES, std::mem::size_of::<CompletedUnit>())?,
            charge_product(2, MAX_JSON_BYTES)?,
            temporary_path_bytes,
        ])?;
        let raster_scratch = charge_sum([
            TILE_RGBA_BYTES,
            loc_grid_bytes,
            MAX_PNG_BYTES as usize,
            temporary_path_bytes,
        ])?;
        let png_decode_scratch = charge_sum([
            MAX_PNG_BYTES as usize,
            TILE_RGBA_BYTES,
            temporary_path_bytes,
        ])?;
        let scratch_bytes = [
            jag_open_scratch,
            jag_member_scratch,
            checkpoint_scratch,
            raster_scratch,
            png_decode_scratch,
        ]
        .into_iter()
        .max()
        .ok_or(MapError::Limit("producer work charge"))?;
        let result_bytes = charge_sum([
            std::mem::size_of::<ImageBakeReport>(),
            charge_product(2 * MAX_IMAGE_TILES, std::mem::size_of::<TileReceipt>())?,
            charge_product(8, std::mem::size_of::<PlaneBounds>())?,
            charge_product(
                2 * usize::from(MAP_PLANES) * (usize::from(MAX_LOD) + 1),
                std::mem::size_of::<LodReceipt>(),
            )?,
        ])?;
        make_work_charge(input_bytes, scratch_bytes, result_bytes)
    }

    pub fn new(
        input: ClientMapInput<'_>,
        partial_dir: Arc<PathBuf>,
        expected_identity: ImageIdentity,
    ) -> Result<Self, MapError> {
        if partial_dir.as_os_str().is_empty() {
            return Err(MapError::Path);
        }
        Ok(Self {
            planner: Some(RasterPlanCursor::new(input, Some(expected_identity))?),
            expected_identity,
            directory: partial_dir,
            phase: RasterBakePhase::Plan,
            context: None,
            plan: None,
            plan_stats: PlanStats::default(),
            progress: RasterProgress {
                stage: RasterStage::ReadingCache,
                completed_tiles: 0,
                total_tiles: 0,
                planning_squares_completed: 0,
                planning_squares_total: None,
            },
            started: Instant::now(),
            groups_builder: BTreeMap::new(),
            groups: Vec::new(),
            base_key_index: 0,
            group_index: 0,
            base_plane_index: 0,
            neighbor_index: 0,
            pending_neighbor: None,
            neighborhood: None,
            adoption_index: 0,
            downsample_index: 0,
            completed: Vec::new(),
            completed_keys: BTreeSet::new(),
            completed_bytes: 0,
            reported_completed: Vec::new(),
            current_tile: None,
            rgba: Vec::new(),
            png: Vec::new(),
            compressed: Vec::new(),
            pixels: Vec::new(),
            parent_children: std::array::from_fn(|_| None),
            parent_child_index: 0,
            parent_row: 0,
            receipts: Vec::new(),
            manifest: None,
            result: None,
            tracker: MemoryTracker::default(),
            checkpoint_enabled: false,
            adopt_existing: true,
            checkpoint_stage: None,
            paused_by_callback: false,
            completed_cursor: false,
        })
    }

    pub fn step<P: WorkPhase, S: ImageTileSink>(
        &mut self,
        context: &mut StepContext<'_, P>,
        sink: &mut S,
    ) -> Result<ProducerStep<ImageBakeReport>, ImageBakeError<S::Error>> {
        let mut keep_running = |_: RasterProgress| true;
        self.step_with(context, sink, &mut keep_running)
    }

    pub fn plan(&self) -> Option<&ImagePlan> {
        self.plan.as_ref()
    }

    pub fn planned_units(&self) -> Option<u32> {
        self.plan
            .as_ref()
            .and_then(|plan| u32::try_from(plan.tiles.len()).ok())
    }

    pub fn progress(&self) -> RasterProgress {
        self.progress
    }

    pub fn take_completed_units(&mut self) -> Vec<CompletedUnit> {
        std::mem::take(&mut self.reported_completed)
    }

    fn enable_checkpointing(&mut self) {
        self.checkpoint_enabled = true;
        self.adopt_existing = false;
    }

    fn resume_completed(
        &mut self,
        completed: Vec<CompletedUnit>,
    ) -> Result<BTreeMap<TileKey, PayloadReceipt>, MapError> {
        let plan = self.plan.as_ref().ok_or(MapError::NotReady)?;
        let planned = self.planned_units().ok_or(MapError::Limit("tile count"))?;
        if completed.len() > usize::try_from(planned).map_err(|_| MapError::Limit("tile count"))? {
            return Err(MapError::Invalid("checkpoint plan count"));
        }
        let mut receipts = BTreeMap::new();
        let mut bytes = 0u64;
        for unit in &completed {
            let UnitKey::Terrain { tile } = unit.key else {
                return Err(MapError::Invalid("image checkpoint unit"));
            };
            if plan.tiles.binary_search(&tile).is_err() {
                return Err(MapError::Invalid("checkpoint unit plan"));
            }
            if receipts.insert(tile, unit.payload).is_some() {
                return Err(MapError::Duplicate("completed tile"));
            }
            bytes = bytes
                .checked_add(u64::from(unit.payload.bytes))
                .ok_or(MapError::Limit("image bytes"))?;
        }
        if bytes > MAX_IMAGE_BYTES {
            return Err(MapError::Limit("image bytes"));
        }
        self.completed_keys = receipts.keys().copied().collect();
        self.completed = completed;
        self.completed.sort_unstable_by_key(|unit| unit.key);
        self.completed_bytes = bytes;
        Ok(receipts)
    }

    fn step_with<P: WorkPhase, S: ImageTileSink, K>(
        &mut self,
        context: &mut StepContext<'_, P>,
        sink: &mut S,
        keep_running: &mut K,
    ) -> Result<ProducerStep<ImageBakeReport>, ImageBakeError<S::Error>>
    where
        K: FnMut(RasterProgress) -> bool,
    {
        if self.completed_cursor {
            return Err(MapError::Invalid("image bake cursor complete").into());
        }
        self.paused_by_callback = false;
        match context.checkpoint() {
            WorkCheckpoint::Continue => {}
            WorkCheckpoint::Yield => return Ok(ProducerStep::Yielded),
            WorkCheckpoint::Cancel(_) => return Ok(ProducerStep::Cancelled),
        }
        loop {
            if matches!(&self.phase, RasterBakePhase::Plan) {
                let planner = self
                    .planner
                    .as_mut()
                    .ok_or(MapError::Invalid("missing raster plan cursor"))?;
                let step = planner.step(context)?;
                if let Some((completed, total)) = planner.planning_squares_progress() {
                    self.progress.planning_squares_completed = completed;
                    self.progress.planning_squares_total = Some(total);
                }
                match step {
                    ProducerStep::Yielded => {
                        self.tracker.observe(planner.peak_tracked_bytes);
                        return Ok(ProducerStep::Yielded);
                    }
                    ProducerStep::Cancelled => return Ok(ProducerStep::Cancelled),
                    ProducerStep::Completed(prepared) => {
                        self.tracker.observe(planner.peak_tracked_bytes);
                        if prepared.plan.identity != self.expected_identity {
                            return Err(MapError::Identity.into());
                        }
                        self.progress.planning_squares_completed = prepared.stats.map_squares;
                        self.progress.planning_squares_total = Some(prepared.stats.map_squares);
                        self.progress.total_tiles = u32::try_from(prepared.plan.tiles.len())
                            .map_err(|_| MapError::Limit("tile count"))?;
                        self.context = Some(prepared.context);
                        self.plan = Some(prepared.plan);
                        self.plan_stats = prepared.stats;
                        self.planner = None;
                        self.phase = RasterBakePhase::BuildGroups;
                        self.observe_memory();
                        return match context.checkpoint() {
                            WorkCheckpoint::Continue | WorkCheckpoint::Yield => {
                                Ok(ProducerStep::Yielded)
                            }
                            WorkCheckpoint::Cancel(_) => Ok(ProducerStep::Cancelled),
                        };
                    }
                }
            }
            let previous_stage = self.progress.stage;
            match self.advance(sink, keep_running)? {
                RasterBakeAction::Transition if self.progress.stage != previous_stage => {
                    return match context.consume_unit() {
                        WorkCheckpoint::Cancel(_) => Ok(ProducerStep::Cancelled),
                        WorkCheckpoint::Continue | WorkCheckpoint::Yield => {
                            Ok(ProducerStep::Yielded)
                        }
                    };
                }
                // consume_unit already checks every completed unit.
                RasterBakeAction::Transition => match context.checkpoint() {
                    WorkCheckpoint::Continue => {}
                    WorkCheckpoint::Yield => return Ok(ProducerStep::Yielded),
                    WorkCheckpoint::Cancel(_) => return Ok(ProducerStep::Cancelled),
                },
                RasterBakeAction::Unit => {
                    self.observe_memory();
                    match context.consume_unit() {
                        WorkCheckpoint::Continue => {
                            if self.progress.stage != previous_stage
                                || matches!(&self.phase, RasterBakePhase::ManifestWrite)
                            {
                                return Ok(ProducerStep::Yielded);
                            }
                        }
                        WorkCheckpoint::Yield => return Ok(ProducerStep::Yielded),
                        WorkCheckpoint::Cancel(_) => return Ok(ProducerStep::Cancelled),
                    }
                }
                RasterBakeAction::Backpressure => return Ok(ProducerStep::Yielded),
                RasterBakeAction::Paused => {
                    self.paused_by_callback = true;
                    self.observe_memory();
                    match context.consume_unit() {
                        WorkCheckpoint::Cancel(_) => return Ok(ProducerStep::Cancelled),
                        WorkCheckpoint::Continue | WorkCheckpoint::Yield => {
                            return Ok(ProducerStep::Yielded)
                        }
                    }
                }
                RasterBakeAction::Completed => match context.checkpoint() {
                    WorkCheckpoint::Continue => {
                        let report = self
                            .result
                            .take()
                            .ok_or(MapError::Invalid("missing completed image bake"))?;
                        self.completed_cursor = true;
                        return Ok(ProducerStep::Completed(report));
                    }
                    WorkCheckpoint::Yield => return Ok(ProducerStep::Yielded),
                    WorkCheckpoint::Cancel(_) => return Ok(ProducerStep::Cancelled),
                },
            }
        }
    }

    fn advance<S: ImageTileSink, K>(
        &mut self,
        sink: &mut S,
        keep_running: &mut K,
    ) -> Result<RasterBakeAction, ImageBakeError<S::Error>>
    where
        K: FnMut(RasterProgress) -> bool,
    {
        let phase = std::mem::replace(&mut self.phase, RasterBakePhase::Complete);
        match phase {
            RasterBakePhase::Plan => Err(MapError::Invalid("unexpected raster plan state").into()),
            RasterBakePhase::BuildGroups => {
                let plan = self.plan.as_ref().ok_or(MapError::NotReady)?;
                if self.base_key_index < plan.base_tiles.len() {
                    let tile = plan.base_tiles[self.base_key_index];
                    self.base_key_index += 1;
                    self.groups_builder
                        .entry((tile.x, tile.z))
                        .or_default()
                        .push(tile.plane);
                    self.phase = RasterBakePhase::BuildGroups;
                    return Ok(RasterBakeAction::Unit);
                }
                self.groups = std::mem::take(&mut self.groups_builder)
                    .into_iter()
                    .map(|((x, z), planes)| (x, z, planes))
                    .collect();
                self.phase = if self.adopt_existing {
                    RasterBakePhase::Adopt
                } else {
                    RasterBakePhase::StartBaseGroup
                };
                Ok(RasterBakeAction::Unit)
            }
            RasterBakePhase::Adopt => {
                let plan = self.plan.as_ref().ok_or(MapError::NotReady)?;
                if self.adoption_index >= plan.tiles.len() {
                    self.phase = RasterBakePhase::StartBaseGroup;
                    return Ok(RasterBakeAction::Transition);
                }
                let key = plan.tiles[self.adoption_index];
                self.adoption_index += 1;
                let recorded = sink.checkpointed(key);
                if let Ok(unit) = completed_from_disk(
                    &self.directory,
                    key,
                    &mut self.compressed,
                    &mut self.pixels,
                ) {
                    if recorded.is_none_or(|receipt| receipt == unit.payload)
                        && !self.completed_keys.contains(&key)
                    {
                        let bytes = self
                            .completed_bytes
                            .checked_add(u64::from(unit.payload.bytes))
                            .ok_or(MapError::Limit("image bytes"))?;
                        if bytes > MAX_IMAGE_BYTES {
                            return Err(MapError::Limit("image bytes").into());
                        }
                        self.completed.push(unit);
                        self.completed.sort_unstable_by_key(|unit| unit.key);
                        self.completed_keys.insert(key);
                        self.completed_bytes = bytes;
                        self.reported_completed.push(unit);
                    }
                }
                self.phase = RasterBakePhase::Adopt;
                Ok(RasterBakeAction::Unit)
            }
            RasterBakePhase::StartBaseGroup => {
                if self.group_index >= self.groups.len() {
                    self.phase = RasterBakePhase::DownsampleCheckpoint;
                    return Ok(RasterBakeAction::Transition);
                }
                let (_, _, planes) = &self.groups[self.group_index];
                let Some((plane_index, plane)) =
                    planes.iter().copied().enumerate().find(|(_, plane)| {
                        !self.completed_keys.contains(&TileKey {
                            plane: *plane,
                            lod: 0,
                            x: self.groups[self.group_index].0,
                            z: self.groups[self.group_index].1,
                        })
                    })
                else {
                    self.group_index += 1;
                    self.phase = RasterBakePhase::StartBaseGroup;
                    return Ok(RasterBakeAction::Unit);
                };
                self.base_plane_index = plane_index;
                self.progress = RasterProgress {
                    stage: RasterStage::BaseTerrain { plane },
                    completed_tiles: self.completed_keys.len() as u32,
                    total_tiles: self.progress.total_tiles,
                    planning_squares_completed: 0,
                    planning_squares_total: None,
                };
                if !keep_running(self.progress) {
                    self.checkpoint_stage = Some(BakeStage::BaseTerrain);
                    self.phase = RasterBakePhase::PauseCheckpoint;
                    return Ok(RasterBakeAction::Transition);
                }
                self.neighborhood = Some(Neighborhood {
                    squares: BTreeMap::new(),
                });
                self.neighbor_index = 0;
                self.phase = RasterBakePhase::LoadNeighborLand;
                Ok(RasterBakeAction::Transition)
            }
            RasterBakePhase::LoadNeighborLand => {
                if self.neighbor_index >= 9 {
                    self.phase = RasterBakePhase::RasterizeBase;
                    return Ok(RasterBakeAction::Transition);
                }
                let (square_x, square_z, _) = self
                    .groups
                    .get(self.group_index)
                    .ok_or(MapError::Invalid("missing base tile group"))?;
                let x = *square_x - 1 + (self.neighbor_index / 3) as i32;
                let z = *square_z - 1 + (self.neighbor_index % 3) as i32;
                let context = self
                    .context
                    .as_mut()
                    .ok_or(MapError::Invalid("missing raster context"))?;
                let Some(entry) = context.reader.entry(x, z) else {
                    self.neighbor_index += 1;
                    self.phase = RasterBakePhase::LoadNeighborLand;
                    return Ok(RasterBakeAction::Transition);
                };
                let land = context.reader.read_land(entry)?;
                self.pending_neighbor = Some((x, z, entry, land));
                self.phase = RasterBakePhase::LoadNeighborLocs;
                Ok(RasterBakeAction::Unit)
            }
            RasterBakePhase::LoadNeighborLocs => {
                let (x, z, entry, land) = self
                    .pending_neighbor
                    .take()
                    .ok_or(MapError::Invalid("missing raster neighbor"))?;
                let locs = self
                    .context
                    .as_mut()
                    .ok_or(MapError::Invalid("missing raster context"))?
                    .reader
                    .read_locs(entry)?;
                self.neighborhood
                    .as_mut()
                    .ok_or(MapError::Invalid("missing raster neighborhood"))?
                    .squares
                    .insert((x, z), SquareData { land, locs });
                self.neighbor_index += 1;
                self.phase = RasterBakePhase::LoadNeighborLand;
                Ok(RasterBakeAction::Unit)
            }
            RasterBakePhase::RasterizeBase => {
                let (_, _, planes) = self
                    .groups
                    .get(self.group_index)
                    .ok_or(MapError::Invalid("missing base tile group"))?;
                if self.base_plane_index >= planes.len() {
                    self.neighborhood = None;
                    self.group_index += 1;
                    self.phase = RasterBakePhase::StartBaseGroup;
                    return Ok(RasterBakeAction::Transition);
                }
                let (x, z, _) = self.groups[self.group_index];
                let key = TileKey {
                    plane: planes[self.base_plane_index],
                    lod: 0,
                    x,
                    z,
                };
                if self.completed_keys.contains(&key) {
                    self.base_plane_index += 1;
                    self.phase = RasterBakePhase::RasterizeBase;
                    return Ok(RasterBakeAction::Transition);
                }
                let (rgba, scratch_bytes) = render_base(
                    self.context
                        .as_ref()
                        .ok_or(MapError::Invalid("missing raster context"))?,
                    self.neighborhood
                        .as_ref()
                        .ok_or(MapError::Invalid("missing raster neighborhood"))?,
                    key,
                )?;
                self.current_tile = Some(key);
                self.rgba = rgba;
                let context_bytes = self
                    .context
                    .as_ref()
                    .ok_or(MapError::Invalid("missing raster context"))?
                    .tracked_bytes();
                let neighborhood_bytes = self
                    .neighborhood
                    .as_ref()
                    .ok_or(MapError::Invalid("missing raster neighborhood"))?
                    .tracked_bytes();
                self.tracker.observe(
                    context_bytes
                        + neighborhood_bytes
                        + self.rgba.capacity() as u64
                        + scratch_bytes as u64,
                );
                self.progress.stage = RasterStage::BaseTerrain { plane: key.plane };
                self.phase = RasterBakePhase::EncodeBase;
                Ok(RasterBakeAction::Unit)
            }
            RasterBakePhase::EncodeBase => {
                self.png = encode_png(&self.rgba)?;
                self.phase = RasterBakePhase::WriteBase;
                Ok(RasterBakeAction::Unit)
            }
            RasterBakePhase::WriteBase => {
                let key = self
                    .current_tile
                    .ok_or(MapError::Invalid("missing base tile"))?;
                self.phase = RasterBakePhase::WriteBase;
                ensure_tile_can_write(key, &self.png, &self.completed_keys, self.completed_bytes)?;
                let payload = match sink.write_tile(key, &self.png) {
                    Ok(Some(payload)) => payload,
                    Ok(None) => return Ok(RasterBakeAction::Backpressure),
                    Err(error) => return Err(ImageBakeError::Sink(error)),
                };
                complete_tile(
                    key,
                    payload,
                    &self.png,
                    &mut self.completed,
                    &mut self.completed_keys,
                    &mut self.completed_bytes,
                )?;
                self.current_tile = None;
                if let Ok(index) = self
                    .completed
                    .binary_search_by_key(&UnitKey::Terrain { tile: key }, |unit| unit.key)
                {
                    self.reported_completed.push(self.completed[index]);
                }
                self.png.clear();
                self.rgba.clear();
                self.base_plane_index += 1;
                if self.checkpoint_enabled {
                    write_checkpoint_if_batch(
                        &self.directory,
                        self.expected_identity,
                        BakeStage::BaseTerrain,
                        self.progress.total_tiles,
                        &self.completed,
                    )?;
                }
                self.progress.completed_tiles = self.completed_keys.len() as u32;
                self.phase = RasterBakePhase::RasterizeBase;
                Ok(RasterBakeAction::Unit)
            }
            RasterBakePhase::DownsampleCheckpoint => {
                if self.checkpoint_enabled {
                    write_checkpoint(
                        &self.directory,
                        self.expected_identity,
                        BakeStage::Downsample,
                        self.progress.total_tiles,
                        &self.completed,
                    )?;
                }
                self.phase = RasterBakePhase::DownsampleScan;
                Ok(RasterBakeAction::Unit)
            }
            RasterBakePhase::DownsampleScan => {
                let plan = self.plan.as_ref().ok_or(MapError::NotReady)?;
                if self.downsample_index >= plan.tiles.len() {
                    self.phase = RasterBakePhase::ManifestReceipts;
                    self.receipts = Vec::with_capacity(self.completed.len());
                    return Ok(RasterBakeAction::Transition);
                }
                let key = plan.tiles[self.downsample_index];
                self.downsample_index += 1;
                if key.lod == 0 || self.completed_keys.contains(&key) {
                    self.phase = RasterBakePhase::DownsampleScan;
                    return Ok(RasterBakeAction::Unit);
                }
                self.progress = RasterProgress {
                    stage: RasterStage::Downsample {
                        plane: key.plane,
                        lod: key.lod,
                    },
                    completed_tiles: self.completed_keys.len() as u32,
                    total_tiles: self.progress.total_tiles,
                    planning_squares_completed: 0,
                    planning_squares_total: None,
                };
                if !keep_running(self.progress) {
                    self.checkpoint_stage = Some(BakeStage::Downsample);
                    self.phase = RasterBakePhase::PauseCheckpoint;
                    return Ok(RasterBakeAction::Transition);
                }
                self.current_tile = Some(key);
                self.rgba.resize(TILE_RGBA_BYTES, 0);
                self.parent_children = std::array::from_fn(|_| None);
                self.parent_child_index = 0;
                self.parent_row = 0;
                self.phase = RasterBakePhase::LoadParentChild;
                Ok(RasterBakeAction::Unit)
            }
            RasterBakePhase::LoadParentChild => {
                if self.parent_child_index >= self.parent_children.len() {
                    self.phase = RasterBakePhase::RasterizeParentRow;
                    return Ok(RasterBakeAction::Transition);
                }
                let parent = self
                    .current_tile
                    .ok_or(MapError::Invalid("missing parent tile"))?;
                let child_index = self.parent_child_index;
                self.parent_child_index += 1;
                let qx = (child_index / 4) as i32 - 1;
                let qy = (child_index % 4) as i32 - 1;
                let child = TileKey {
                    plane: parent.plane,
                    lod: parent.lod - 1,
                    x: parent.x * 2 + qx,
                    z: parent.z * 2 + 1 - qy,
                };
                let plan = self.plan.as_ref().ok_or(MapError::NotReady)?;
                if plan.tiles.binary_search(&child).is_ok() {
                    let receipt = self
                        .completed
                        .binary_search_by_key(&UnitKey::Terrain { tile: child }, |unit| unit.key)
                        .ok()
                        .map(|index| self.completed[index].payload)
                        .ok_or(MapError::Invalid("missing completed child"))?;
                    read_bounded(
                        &self.directory.join(child.relative_path()?),
                        MAX_PNG_BYTES as usize,
                        &mut self.compressed,
                    )?;
                    receipt.verify(&self.compressed, MAX_PNG_BYTES)?;
                    self.parent_children[child_index] = Some(decode_png(&self.compressed)?);
                }
                self.phase = RasterBakePhase::LoadParentChild;
                Ok(RasterBakeAction::Unit)
            }
            RasterBakePhase::RasterizeParentRow => {
                if self.parent_row >= TILE_PIXELS as usize {
                    self.phase = RasterBakePhase::EncodeParent;
                    return Ok(RasterBakeAction::Transition);
                }
                render_parent_row(
                    &self.parent_children,
                    &mut self.rgba,
                    self.parent_row as i32,
                );
                self.parent_row += 1;
                self.phase = RasterBakePhase::RasterizeParentRow;
                Ok(RasterBakeAction::Unit)
            }
            RasterBakePhase::EncodeParent => {
                self.png = encode_png(&self.rgba)?;
                self.phase = RasterBakePhase::WriteParent;
                Ok(RasterBakeAction::Unit)
            }
            RasterBakePhase::WriteParent => {
                let key = self
                    .current_tile
                    .ok_or(MapError::Invalid("missing parent tile"))?;
                self.phase = RasterBakePhase::WriteParent;
                ensure_tile_can_write(key, &self.png, &self.completed_keys, self.completed_bytes)?;
                let payload = match sink.write_tile(key, &self.png) {
                    Ok(Some(payload)) => payload,
                    Ok(None) => return Ok(RasterBakeAction::Backpressure),
                    Err(error) => return Err(ImageBakeError::Sink(error)),
                };
                complete_tile(
                    key,
                    payload,
                    &self.png,
                    &mut self.completed,
                    &mut self.completed_keys,
                    &mut self.completed_bytes,
                )?;
                self.current_tile = None;
                if let Ok(index) = self
                    .completed
                    .binary_search_by_key(&UnitKey::Terrain { tile: key }, |unit| unit.key)
                {
                    self.reported_completed.push(self.completed[index]);
                }
                self.png.clear();
                self.rgba.clear();
                self.parent_children.fill(None);
                self.parent_child_index = 0;
                self.parent_row = 0;
                if self.checkpoint_enabled {
                    write_checkpoint_if_batch(
                        &self.directory,
                        self.expected_identity,
                        BakeStage::Downsample,
                        self.progress.total_tiles,
                        &self.completed,
                    )?;
                }
                self.progress.completed_tiles = self.completed_keys.len() as u32;
                self.phase = RasterBakePhase::DownsampleScan;
                Ok(RasterBakeAction::Unit)
            }
            RasterBakePhase::ManifestReceipts => {
                if self.receipts.len() >= self.completed.len() {
                    self.phase = RasterBakePhase::ManifestBuild;
                    return Ok(RasterBakeAction::Transition);
                }
                let unit = self.completed[self.receipts.len()];
                let UnitKey::Terrain { tile } = unit.key else {
                    return Err(MapError::Invalid("image checkpoint unit").into());
                };
                self.receipts.push(TileReceipt {
                    key: tile,
                    payload: unit.payload,
                });
                self.phase = RasterBakePhase::ManifestReceipts;
                Ok(RasterBakeAction::Unit)
            }
            RasterBakePhase::ManifestBuild => {
                let plan = self.plan.as_ref().ok_or(MapError::NotReady)?;
                if self.completed.len() != plan.tiles.len() {
                    return Err(MapError::Invalid("incomplete image plan").into());
                }
                self.progress = RasterProgress {
                    stage: RasterStage::Publishing,
                    completed_tiles: self.progress.total_tiles,
                    total_tiles: self.progress.total_tiles,
                    planning_squares_completed: 0,
                    planning_squares_total: None,
                };
                let identity = plan.identity;
                self.manifest = Some(ImageManifest {
                    schema: IMAGE_SCHEMA,
                    identity,
                    key: identity.key()?,
                    extent: plan.extent,
                    planes: Rows::new(plan.planes.clone())?,
                    max_lod: plan.max_lod,
                    interior: TILE_INTERIOR,
                    gutter: TILE_GUTTER,
                    color: ColorFormat::Rgba8Unorm,
                    tiles: Rows::new(std::mem::take(&mut self.receipts))?,
                });
                self.phase = RasterBakePhase::ManifestWrite;
                Ok(RasterBakeAction::Unit)
            }
            RasterBakePhase::ManifestWrite => {
                self.progress = RasterProgress {
                    stage: RasterStage::Publishing,
                    completed_tiles: self.progress.total_tiles,
                    total_tiles: self.progress.total_tiles,
                    planning_squares_completed: 0,
                    planning_squares_total: None,
                };
                let manifest = self
                    .manifest
                    .as_ref()
                    .ok_or(MapError::Invalid("missing image manifest"))?;
                let manifest_bytes = manifest.encode()?;
                let retained_bytes = self.observe_memory();
                self.tracker
                    .observe(retained_bytes + manifest_bytes.capacity() as u64);
                if self.checkpoint_enabled {
                    atomic_write(&self.directory.join("manifest.json"), &manifest_bytes)?;
                    write_checkpoint(
                        &self.directory,
                        self.expected_identity,
                        BakeStage::Publishing,
                        self.progress.total_tiles,
                        &self.completed,
                    )?;
                }
                let _ = keep_running(self.progress);
                self.result = Some(ImageBakeReport {
                    manifest: self
                        .manifest
                        .take()
                        .ok_or(MapError::Invalid("missing image manifest"))?,
                    metrics: metrics_from(
                        &self.completed,
                        self.plan_stats,
                        self.tracker.peak,
                        self.started.elapsed(),
                    ),
                });
                self.phase = RasterBakePhase::Complete;
                Ok(RasterBakeAction::Unit)
            }
            RasterBakePhase::PauseCheckpoint => {
                if let Some(stage) = self.checkpoint_stage.take() {
                    if self.checkpoint_enabled {
                        write_checkpoint(
                            &self.directory,
                            self.expected_identity,
                            stage,
                            self.progress.total_tiles,
                            &self.completed,
                        )?;
                    }
                    self.phase = match stage {
                        BakeStage::BaseTerrain => RasterBakePhase::StartBaseGroup,
                        BakeStage::Downsample => RasterBakePhase::DownsampleScan,
                        BakeStage::Catalogue | BakeStage::Publishing => {
                            return Err(MapError::Invalid("invalid image pause stage").into());
                        }
                    };
                    return Ok(RasterBakeAction::Paused);
                }
                Err(MapError::Invalid("missing image pause stage").into())
            }
            RasterBakePhase::Complete => Ok(RasterBakeAction::Completed),
        }
    }

    fn observe_memory(&mut self) -> u64 {
        let mut bytes = self
            .context
            .as_ref()
            .map_or(0, RasterContext::tracked_bytes);
        if let Some(plan) = &self.plan {
            bytes += (plan.tiles.capacity() + plan.base_tiles.capacity()) as u64
                * std::mem::size_of::<TileKey>() as u64;
            bytes += plan.planes.capacity() as u64 * std::mem::size_of::<PlaneBounds>() as u64;
        }
        if let Some(neighborhood) = &self.neighborhood {
            bytes += neighborhood.tracked_bytes();
        }
        bytes += self.completed.capacity() as u64 * std::mem::size_of::<CompletedUnit>() as u64;
        bytes +=
            self.reported_completed.capacity() as u64 * std::mem::size_of::<CompletedUnit>() as u64;
        bytes += self.completed_keys.len() as u64
            * (std::mem::size_of::<TileKey>() as u64 + 3 * std::mem::size_of::<usize>() as u64);
        bytes += self.groups.capacity() as u64 * std::mem::size_of::<(i32, i32, Vec<u8>)>() as u64;
        bytes += self.groups_builder.len() as u64
            * (std::mem::size_of::<(i32, i32)>() as u64
                + std::mem::size_of::<Vec<u8>>() as u64
                + 3 * std::mem::size_of::<usize>() as u64);
        bytes += self
            .groups
            .iter()
            .map(|(_, _, planes)| planes.capacity() as u64)
            .sum::<u64>();
        bytes += self
            .groups_builder
            .values()
            .map(|planes| planes.capacity() as u64)
            .sum::<u64>();
        bytes += self.rgba.capacity() as u64
            + self.png.capacity() as u64
            + self.compressed.capacity() as u64
            + self.pixels.capacity() as u64
            + self
                .parent_children
                .iter()
                .flatten()
                .map(|child| child.capacity() as u64)
                .sum::<u64>();
        bytes += self.receipts.capacity() as u64 * std::mem::size_of::<TileReceipt>() as u64;
        if let Some(manifest) = &self.manifest {
            bytes +=
                manifest.tiles.as_slice().len() as u64 * std::mem::size_of::<TileReceipt>() as u64;
            bytes +=
                manifest.planes.as_slice().len() as u64 * std::mem::size_of::<PlaneBounds>() as u64;
        }
        if let Some(result) = &self.result {
            bytes += result.manifest.tiles.as_slice().len() as u64
                * std::mem::size_of::<TileReceipt>() as u64;
            bytes += result.manifest.planes.as_slice().len() as u64
                * std::mem::size_of::<PlaneBounds>() as u64;
            bytes +=
                result.metrics.lods.capacity() as u64 * std::mem::size_of::<LodReceipt>() as u64;
        }
        if let Some((_, _, _, land)) = &self.pending_neighbor {
            bytes += land.cells_capacity_bytes() as u64;
        }
        self.tracker.observe(bytes);
        if let Some(result) = &mut self.result {
            result.metrics.peak_tracked_bytes = self.tracker.peak;
        }
        bytes
    }

    fn was_paused_by_callback(&self) -> bool {
        self.paused_by_callback
    }
}
struct SquareData {
    land: LandSquare,
    locs: Vec<LocPlacement>,
}

struct Neighborhood {
    squares: BTreeMap<(i32, i32), SquareData>,
}

#[derive(Debug, Clone, Copy)]
struct MinimapCell {
    effective_plane: u8,
    visibility_flags: u8,
    cell: GroundCell,
}

impl Neighborhood {
    fn square(&self, world_x: i32, world_z: i32) -> Option<(&SquareData, u8, u8)> {
        let square_x = world_x.div_euclid(64);
        let square_z = world_z.div_euclid(64);
        let square = self.squares.get(&(square_x, square_z))?;
        Some((
            square,
            world_x.rem_euclid(64) as u8,
            world_z.rem_euclid(64) as u8,
        ))
    }

    fn minimap_cell(
        &self,
        plane: u8,
        world_x: i32,
        world_z: i32,
        above: bool,
    ) -> Option<MinimapCell> {
        let (square, x, z) = self.square(world_x, world_z)?;
        let link_below = square.land.link_below(x, z);
        let effective_plane = plane.checked_add(u8::from(above))?;
        if effective_plane >= 4 {
            return None;
        }
        let raw_plane = effective_plane.checked_add(u8::from(link_below))?;
        if raw_plane >= 4 {
            return None;
        }
        Some(MinimapCell {
            effective_plane,
            visibility_flags: square.land.cell(effective_plane, x, z).flags,
            cell: square.land.cell(raw_plane, x, z),
        })
    }

    fn tracked_bytes(&self) -> u64 {
        self.squares
            .values()
            .map(|square| {
                square.land.cells_capacity_bytes() as u64
                    + (square.locs.capacity() * std::mem::size_of::<LocPlacement>()) as u64
            })
            .sum()
    }
}

fn render_base(
    context: &RasterContext,
    neighborhood: &Neighborhood,
    key: TileKey,
) -> Result<(Vec<u8>, usize), MapError> {
    let mut rgba = vec![0; TILE_RGBA_BYTES];
    let west = key.x * 64;
    let north = (key.z + 1) * 64;
    let grid_west = west - 1 - GRID_MARGIN;
    let grid_south = key.z * 64 - 1 - GRID_MARGIN;
    let grid_size = 66 + GRID_MARGIN * 2;
    let mut grid = Vec::with_capacity((grid_size * grid_size) as usize);

    // The native minimap paints the normalized plane first, then paints the
    // next raw plane over it only where VIS_BELOW requests that composition.
    for above in [false, true] {
        grid.clear();
        for x in 0..grid_size {
            for z in 0..grid_size {
                grid.push(neighborhood.minimap_cell(
                    key.plane,
                    grid_west + x,
                    grid_south + z,
                    above,
                ));
            }
        }
        let grid_cell = |world_x: i32, world_z: i32| -> Option<MinimapCell> {
            let x = world_x - grid_west;
            let z = world_z - grid_south;
            if x < 0 || z < 0 || x >= grid_size || z >= grid_size {
                return None;
            }
            grid[(x * grid_size + z) as usize]
        };
        let cell_at = |world_x: i32, world_z: i32| -> Option<GroundCell> {
            grid_cell(world_x, world_z).map(|layer| layer.cell)
        };

        for world_x in west - 1..=west + 64 {
            for world_z in key.z * 64 - 1..=key.z * 64 + 64 {
                let Some(layer) = grid_cell(world_x, world_z) else {
                    continue;
                };
                if minimap_plane(layer.effective_plane, layer.visibility_flags) != Some(key.plane)
                    || (layer.cell.underlay == 0 && layer.cell.overlay == 0)
                {
                    continue;
                }
                let underlay =
                    blended_underlay(&context.definitions, &cell_at, world_x, world_z, layer.cell);
                let overlay =
                    overlay_colour(&context.definitions, &context.assets, layer.cell.overlay);
                let pixel_x = 1 + (world_x - west) * 4;
                let pixel_y = 1 + (north - world_z - 1) * 4;
                draw_floor_cell(
                    &mut rgba,
                    pixel_x,
                    pixel_y,
                    underlay,
                    overlay,
                    layer.cell.overlay_shape,
                    layer.cell.overlay_rotation,
                );
            }
        }
    }
    let scratch_bytes = grid.capacity() * std::mem::size_of::<Option<MinimapCell>>();

    // Detail uses the same two native passes so walls and mapscenes on hidden
    // tiles neither leak onto their raw plane nor overpaint VIS_BELOW detail.
    for draw_vis_below in [false, true] {
        for square in neighborhood.squares.values() {
            for placement in &square.locs {
                let link_below = square.land.link_below(placement.x, placement.z);
                let Some(effective_plane) = effective_plane(placement.plane, link_below) else {
                    continue;
                };
                let flags = square
                    .land
                    .cell(effective_plane, placement.x, placement.z)
                    .flags;
                if (i32::from(flags) & MapFlag::VIS_BELOW != 0) != draw_vis_below
                    || minimap_plane(effective_plane, flags) != Some(key.plane)
                {
                    continue;
                }
                let Some(definition) = context.definitions.locs.get(placement.id as usize) else {
                    return Err(MapError::Invalid("location definition id"));
                };
                let world_x = square.land.square_x * 64 + i32::from(placement.x);
                let world_z = square.land.square_z * 64 + i32::from(placement.z);
                draw_location(
                    &mut rgba,
                    west,
                    north,
                    world_x,
                    world_z,
                    *placement,
                    definition,
                    &context.assets,
                )?;
            }
        }
    }
    Ok((rgba, scratch_bytes))
}

fn blended_underlay(
    definitions: &Definitions,
    cell_at: &impl Fn(i32, i32) -> Option<GroundCell>,
    world_x: i32,
    world_z: i32,
    centre: GroundCell,
) -> Option<u32> {
    if centre.underlay == 0 {
        return None;
    }
    let mut hue = 0i64;
    let mut saturation = 0i64;
    let mut lightness = 0i64;
    let mut chroma = 0i64;
    let mut count = 0i64;
    let mut direct_red = 0i64;
    let mut direct_green = 0i64;
    let mut direct_blue = 0i64;
    for dx in -4..=5 {
        for dz in -4..=5 {
            let Some(cell) = cell_at(world_x + dx, world_z + dz) else {
                continue;
            };
            let Some(floor) = floor(definitions, cell.underlay) else {
                continue;
            };
            hue += i64::from(floor.underlay_hue);
            saturation += i64::from(floor.saturation);
            lightness += i64::from(floor.lightness);
            chroma += i64::from(floor.chroma);
            direct_red += i64::from((floor.colour >> 16) & 0xff);
            direct_green += i64::from((floor.colour >> 8) & 0xff);
            direct_blue += i64::from(floor.colour & 0xff);
            count += 1;
        }
    }
    if count == 0 {
        return None;
    }
    let rgb = if chroma > 0 {
        hsl_to_rgb(
            ((hue * 256) / chroma) as i32,
            (saturation / count) as i32,
            (lightness / count) as i32,
        )
    } else {
        ((direct_red / count) as u32) << 16
            | ((direct_green / count) as u32) << 8
            | (direct_blue / count) as u32
    };
    let west = cell_at(world_x - 1, world_z).unwrap_or(centre).height;
    let east = cell_at(world_x + 1, world_z).unwrap_or(centre).height;
    let south = cell_at(world_x, world_z - 1).unwrap_or(centre).height;
    let north = cell_at(world_x, world_z + 1).unwrap_or(centre).height;
    Some(shade(rgb, east - west, north - south))
}

fn floor(definitions: &Definitions, id: u8) -> Option<&FloorDefinition> {
    id.checked_sub(1)
        .and_then(|index| definitions.floors.get(index as usize))
}

fn overlay_colour(definitions: &Definitions, assets: &RasterAssets, id: u8) -> Option<u32> {
    let floor = floor(definitions, id)?;
    if let Some(texture) = floor.texture {
        let colour = assets
            .texture_colours
            .get(texture as usize)
            .copied()
            .unwrap_or(0);
        return (colour != 0).then_some(colour);
    }
    if floor.colour == 0xff00ff {
        None
    } else {
        Some(floor.colour)
    }
}

fn draw_floor_cell(
    rgba: &mut [u8],
    x: i32,
    y: i32,
    underlay: Option<u32>,
    overlay: Option<u32>,
    shape: u8,
    rotation: u8,
) {
    let shape = usize::from(shape.saturating_add(1).min(12));
    let rotation = usize::from(rotation & 3);
    for py in 0..4 {
        for px in 0..4 {
            let index = py * 4 + px;
            let use_overlay = MINIMAP_SHAPE[shape][MINIMAP_ROTATE[rotation][index]] != 0;
            let colour = match overlay {
                Some(overlay) if use_overlay => Some(overlay),
                _ => underlay,
            };
            if let Some(colour) = colour {
                put_rgb(rgba, x + px as i32, y + py as i32, colour);
            }
        }
    }
}

#[expect(clippy::too_many_arguments, reason = "Drawing needs the buffer, world geometry, placement, definition, and assets.")]
fn draw_location(
    rgba: &mut [u8],
    west: i32,
    north: i32,
    world_x: i32,
    world_z: i32,
    placement: LocPlacement,
    definition: &RasterLocDefinition,
    assets: &RasterAssets,
) -> Result<(), MapError> {
    let base_x = 1 + (world_x - west) * 4;
    let base_y = 1 + (north - world_z - 1) * 4;
    if let Some(mapscene) = definition
        .mapscene
        .filter(|_| !matches!(placement.shape, 4..=8))
    {
        if let Some(sprite) = assets
            .mapscenes
            .get(mapscene as usize)
            .and_then(Option::as_ref)
        {
            let x = base_x + (i32::from(definition.width) * 4 - sprite.wi) / 2 + sprite.xof;
            let y = 1
                + (north - world_z - i32::from(definition.length)) * 4
                + (i32::from(definition.length) * 4 - sprite.hi) / 2
                + sprite.yof;
            plot_sprite(rgba, sprite, x, y)?;
        }
        return Ok(());
    }
    let colour = if definition.active {
        FIXED_ACTIVE_WALL_RGB
    } else {
        FIXED_WALL_RGB
    };
    match placement.shape {
        0 => draw_straight_wall(rgba, base_x, base_y, placement.rotation, colour),
        2 => {
            draw_straight_wall(rgba, base_x, base_y, placement.rotation, colour);
            draw_straight_wall(rgba, base_x, base_y, (placement.rotation + 1) & 3, colour);
        }
        3 => draw_corner(rgba, base_x, base_y, placement.rotation, colour),
        9 => draw_diagonal(rgba, base_x, base_y, placement.rotation, colour),
        _ => {}
    }
    Ok(())
}

fn draw_straight_wall(rgba: &mut [u8], x: i32, y: i32, rotation: u8, colour: u32) {
    match rotation & 3 {
        0 => (0..4).for_each(|dy| put_rgb(rgba, x, y + dy, colour)),
        1 => (0..4).for_each(|dx| put_rgb(rgba, x + dx, y, colour)),
        2 => (0..4).for_each(|dy| put_rgb(rgba, x + 3, y + dy, colour)),
        _ => (0..4).for_each(|dx| put_rgb(rgba, x + dx, y + 3, colour)),
    }
}

fn draw_corner(rgba: &mut [u8], x: i32, y: i32, rotation: u8, colour: u32) {
    let (dx, dy) = match rotation & 3 {
        0 => (0, 0),
        1 => (3, 0),
        2 => (3, 3),
        _ => (0, 3),
    };
    put_rgb(rgba, x + dx, y + dy, colour);
}

fn draw_diagonal(rgba: &mut [u8], x: i32, y: i32, rotation: u8, colour: u32) {
    for offset in 0..4 {
        let row = if matches!(rotation & 3, 0 | 2) {
            3 - offset
        } else {
            offset
        };
        put_rgb(rgba, x + offset, y + row, colour);
    }
}

fn plot_sprite(rgba: &mut [u8], sprite: &Pix8, x: i32, y: i32) -> Result<(), MapError> {
    if sprite.wi <= 0 || sprite.hi <= 0 || sprite.wi > 256 || sprite.hi > 256 {
        return Err(MapError::Limit("mapscene dimensions"));
    }
    let expected = usize::try_from(sprite.wi * sprite.hi)
        .map_err(|_| MapError::Invalid("mapscene dimensions"))?;
    if sprite.data.len() != expected {
        return Err(MapError::Invalid("mapscene pixels"));
    }
    for sy in 0..sprite.hi {
        for sx in 0..sprite.wi {
            let palette_index = sprite.data[(sx + sy * sprite.wi) as usize] as u8 as usize;
            if palette_index == 0 {
                continue;
            }
            let colour = *sprite
                .bpal
                .get(palette_index)
                .ok_or(MapError::Invalid("mapscene palette"))? as u32;
            put_rgb(rgba, x + sx, y + sy, colour);
        }
    }
    Ok(())
}

fn put_rgb(rgba: &mut [u8], x: i32, y: i32, colour: u32) {
    if x < 0 || y < 0 || x >= TILE_PIXELS as i32 || y >= TILE_PIXELS as i32 {
        return;
    }
    let offset = (y as usize * TILE_PIXELS as usize + x as usize) * 4;
    rgba[offset] = (colour >> 16) as u8;
    rgba[offset + 1] = (colour >> 8) as u8;
    rgba[offset + 2] = colour as u8;
    rgba[offset + 3] = 255;
}

fn render_parent_row(children: &[Option<Vec<u8>>; 16], output: &mut [u8], y: i32) {
    for x in 0..TILE_PIXELS as i32 {
        let mut samples = [[0u8; 4]; 4];
        for sy in 0..2 {
            for sx in 0..2 {
                let source_x = 2 * (x - 1) + sx;
                let source_y = 2 * (y - 1) + sy;
                let qx = source_x.div_euclid(256);
                let qy = source_y.div_euclid(256);
                let slot = ((qx + 1) * 4 + (qy + 1)) as usize;
                if let Some(child) = children.get(slot).and_then(Option::as_ref) {
                    let local_x = source_x.rem_euclid(256) + 1;
                    let local_y = source_y.rem_euclid(256) + 1;
                    let offset = ((local_y * TILE_PIXELS as i32 + local_x) * 4) as usize;
                    samples[(sy * 2 + sx) as usize].copy_from_slice(&child[offset..offset + 4]);
                }
            }
        }
        let pixel = average_rgba(samples);
        let offset = ((y * TILE_PIXELS as i32 + x) * 4) as usize;
        output[offset..offset + 4].copy_from_slice(&pixel);
    }
}

fn average_rgba(samples: [[u8; 4]; 4]) -> [u8; 4] {
    let alpha_sum: u32 = samples.iter().map(|pixel| u32::from(pixel[3])).sum();
    if alpha_sum == 0 {
        return [0; 4];
    }
    let mut output = [0; 4];
    for channel in 0..3 {
        let weighted: u32 = samples
            .iter()
            .map(|pixel| u32::from(pixel[channel]) * u32::from(pixel[3]))
            .sum();
        output[channel] = ((weighted + alpha_sum / 2) / alpha_sum) as u8;
    }
    output[3] = ((alpha_sum + 2) / 4) as u8;
    output
}

fn encode_png(rgba: &[u8]) -> Result<Vec<u8>, MapError> {
    if rgba.len() != TILE_RGBA_BYTES {
        return Err(MapError::Invalid("RGBA tile length"));
    }
    let mut bytes = Vec::new();
    {
        let mut encoder = Encoder::new(&mut bytes, TILE_PIXELS, TILE_PIXELS);
        encoder.set_color(ColorType::Rgba);
        encoder.set_depth(BitDepth::Eight);
        encoder.set_compression(Compression::High);
        encoder.set_filter(Filter::Adaptive);
        let mut writer = encoder
            .write_header()
            .map_err(|_| MapError::Invalid("PNG encode header"))?;
        writer
            .write_image_data(rgba)
            .map_err(|_| MapError::Invalid("PNG encode data"))?;
    }
    if bytes.len() > MAX_PNG_BYTES as usize {
        return Err(MapError::Limit("compressed PNG bytes"));
    }
    Ok(bytes)
}

fn decode_png(bytes: &[u8]) -> Result<Vec<u8>, MapError> {
    let mut output = Vec::new();
    decode_png_into(bytes, &mut output)?;
    Ok(output)
}

/// Decode one 258×258 RGBA8 tile into `output`, reusing its allocation.
/// Chunk CRCs and the zlib Adler-32 are checked, and the stream must reach
/// IEND, so a truncated or damaged file is an error.
fn decode_png_into(bytes: &[u8], output: &mut Vec<u8>) -> Result<(), MapError> {
    let mut options = png::DecodeOptions::default();
    options.set_ignore_crc(false);
    options.set_ignore_adler32(false);
    let decoder = Decoder::new_with_options(Cursor::new(bytes), options);
    let mut reader = decoder
        .read_info()
        .map_err(|_| MapError::Invalid("PNG decode header"))?;
    let info = reader.info();
    if info.width != TILE_PIXELS
        || info.height != TILE_PIXELS
        || info.color_type != ColorType::Rgba
        || info.bit_depth != BitDepth::Eight
    {
        return Err(MapError::Invalid("PNG child format"));
    }
    let size = reader
        .output_buffer_size()
        .ok_or(MapError::Limit("PNG decode bytes"))?;
    if size != TILE_RGBA_BYTES {
        return Err(MapError::Invalid("PNG decoded length"));
    }
    output.resize(size, 0);
    let frame = reader
        .next_frame(output)
        .map_err(|_| MapError::Invalid("PNG decode data"))?;
    if frame.buffer_size() != TILE_RGBA_BYTES {
        return Err(MapError::Invalid("PNG frame length"));
    }
    reader
        .finish()
        .map_err(|_| MapError::Invalid("PNG trailing chunks"))?;
    Ok(())
}

struct AtomicImageTileSink<'a, F> {
    directory: &'a Path,
    checkpointed: F,
}

impl<'a, F> AtomicImageTileSink<'a, F>
where
    F: FnMut(TileKey) -> Option<PayloadReceipt>,
{
    fn new(directory: &'a Path, checkpointed: F) -> Self {
        Self {
            directory,
            checkpointed,
        }
    }
}

impl<F> ImageTileSink for AtomicImageTileSink<'_, F>
where
    F: FnMut(TileKey) -> Option<PayloadReceipt>,
{
    type Error = MapError;

    fn checkpointed(&mut self, key: TileKey) -> Option<PayloadReceipt> {
        (self.checkpointed)(key)
    }

    fn write_tile(
        &mut self,
        key: TileKey,
        png: &[u8],
    ) -> Result<Option<PayloadReceipt>, Self::Error> {
        let payload = PayloadReceipt {
            bytes: png.len() as u32,
            sha256: Digest::of(png),
        };
        atomic_write(&self.directory.join(key.relative_path()?), png)?;
        Ok(Some(payload))
    }
}

fn ensure_tile_can_write(
    key: TileKey,
    png: &[u8],
    completed_keys: &BTreeSet<TileKey>,
    completed_bytes: u64,
) -> Result<(), MapError> {
    key.bounds()?;
    if completed_keys.contains(&key) {
        return Err(MapError::Duplicate("completed tile"));
    }
    if png.is_empty() {
        return Err(MapError::Invalid("empty payload"));
    }
    if png.len() > MAX_PNG_BYTES as usize {
        return Err(MapError::Limit("payload bytes"));
    }
    let Some(projected_bytes) = completed_bytes.checked_add(png.len() as u64) else {
        return Err(MapError::Limit("image bytes"));
    };
    if projected_bytes > MAX_IMAGE_BYTES {
        return Err(MapError::Limit("image bytes"));
    }
    Ok(())
}

fn complete_tile(
    key: TileKey,
    payload: PayloadReceipt,
    png: &[u8],
    completed: &mut Vec<CompletedUnit>,
    completed_keys: &mut BTreeSet<TileKey>,
    completed_bytes: &mut u64,
) -> Result<(), MapError> {
    let receipt = TileReceipt { key, payload };
    let Some(projected_bytes) = completed_bytes.checked_add(u64::from(payload.bytes)) else {
        return Err(MapError::Limit("image bytes"));
    };
    if projected_bytes > MAX_IMAGE_BYTES {
        return Err(MapError::Limit("image bytes"));
    }
    receipt.verify_png(png)?;
    if completed_keys.contains(&key) {
        return Err(MapError::Duplicate("completed tile"));
    }
    completed.push(CompletedUnit {
        key: UnitKey::Terrain { tile: key },
        payload,
    });
    completed.sort_unstable_by_key(|unit| unit.key);
    completed_keys.insert(key);
    *completed_bytes = projected_bytes;
    Ok(())
}

fn write_checkpoint_if_batch(
    directory: &Path,
    identity: ImageIdentity,
    stage: BakeStage,
    planned_units: u32,
    completed: &[CompletedUnit],
) -> Result<(), MapError> {
    if completed.len().is_multiple_of(CHECKPOINT_BATCH_TILES) {
        write_checkpoint(directory, identity, stage, planned_units, completed)?;
    }
    Ok(())
}

fn write_checkpoint(
    directory: &Path,
    identity: ImageIdentity,
    stage: BakeStage,
    planned_units: u32,
    completed: &[CompletedUnit],
) -> Result<(), MapError> {
    let checkpoint = Checkpoint {
        schema: super::cache::CHECKPOINT_SCHEMA,
        identity: ArtifactIdentity::Image(identity),
        stage,
        planned_units,
        completed: Rows::new(completed.to_vec())?,
    };
    atomic_write(&directory.join("checkpoint.json"), &checkpoint.encode()?)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), MapError> {
    let parent = path.parent().ok_or(MapError::Path)?;
    fs::create_dir_all(parent)?;
    let file_name = path.file_name().ok_or(MapError::Path)?.to_string_lossy();
    let temporary = parent.join(format!(".{file_name}.tmp"));
    let mut file = File::create(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary, path)?;
    Ok(())
}

fn read_bounded(path: &Path, limit: usize, buffer: &mut Vec<u8>) -> Result<(), MapError> {
    let mut file = File::open(path)?;
    let length = file.metadata()?.len();
    if length == 0 || length > limit as u64 {
        return Err(MapError::Limit("generated tile bytes"));
    }
    buffer.resize(length as usize, 0);
    file.read_exact(buffer)?;
    Ok(())
}

fn metrics_from(
    completed: &[CompletedUnit],
    plan: PlanStats,
    peak_tracked_bytes: u64,
    elapsed: Duration,
) -> RasterMetrics {
    let mut by_lod = BTreeMap::<(u8, u8), LodReceipt>::new();
    let mut total = 0u64;
    for unit in completed {
        if let UnitKey::Terrain { tile } = unit.key {
            let receipt = by_lod.entry((tile.plane, tile.lod)).or_insert(LodReceipt {
                plane: tile.plane,
                lod: tile.lod,
                ..LodReceipt::default()
            });
            receipt.tiles += 1;
            receipt.compressed_bytes += u64::from(unit.payload.bytes);
            total += u64::from(unit.payload.bytes);
        }
    }
    RasterMetrics {
        plan,
        lods: by_lod.into_values().collect(),
        total_compressed_bytes: total,
        peak_tracked_bytes,
        elapsed,
    }
}

fn mark_mapscene_tiles(
    tiles: &mut BTreeSet<TileKey>,
    extent: WorldBounds,
    entry: MapIndexEntry,
    placement: LocPlacement,
    definition: &RasterLocDefinition,
    assets: &RasterAssets,
    plane: u8,
) -> Result<(), MapError> {
    if matches!(placement.shape, 4..=8) {
        return Ok(());
    }
    let Some(mapscene) = definition.mapscene else {
        return Ok(());
    };
    let Some(sprite) = assets
        .mapscenes
        .get(mapscene as usize)
        .and_then(Option::as_ref)
    else {
        return Ok(());
    };
    if sprite.wi <= 0 || sprite.hi <= 0 || sprite.wi > 256 || sprite.hi > 256 {
        return Err(MapError::Limit("mapscene dimensions"));
    }
    let world_x = i32::from(entry.square_x) * 64 + i32::from(placement.x);
    let world_z = i32::from(entry.square_z) * 64 + i32::from(placement.z);
    let left = world_x * 4 + (i32::from(definition.width) * 4 - sprite.wi) / 2 + sprite.xof;
    let north = (world_z + i32::from(definition.length)) * 4
        - (i32::from(definition.length) * 4 - sprite.hi) / 2
        - sprite.yof;
    let min_x = left.div_euclid(256);
    let max_x = (left + sprite.wi - 1).div_euclid(256);
    let min_z = (north - sprite.hi).div_euclid(256);
    let max_z = (north - 1).div_euclid(256);
    for x in min_x..=max_x {
        for z in min_z..=max_z {
            let key = TileKey {
                plane,
                lod: 0,
                x,
                z,
            };
            if extent.intersects(key)? {
                tiles.insert(key);
            }
        }
    }
    Ok(())
}

fn draws_native_detail(shape: u8) -> bool {
    matches!(shape, 0 | 2 | 3 | 9)
}

fn hsl_to_rgb(hue: i32, saturation: i32, lightness: i32) -> u32 {
    let h = f64::from(hue.rem_euclid(256)) / 256.0;
    let s = f64::from(saturation.clamp(0, 255)) / 256.0;
    let l = f64::from(lightness.clamp(0, 255)) / 256.0;
    let (red, green, blue) = if s == 0.0 {
        (l, l, l)
    } else {
        let q = if l < 0.5 {
            l * (1.0 + s)
        } else {
            l + s - l * s
        };
        let p = 2.0 * l - q;
        (
            hue_channel(p, q, h + 1.0 / 3.0),
            hue_channel(p, q, h),
            hue_channel(p, q, h - 1.0 / 3.0),
        )
    };
    ((red * 255.0) as u32) << 16 | ((green * 255.0) as u32) << 8 | (blue * 255.0) as u32
}

fn hue_channel(p: f64, q: f64, mut t: f64) -> f64 {
    if t < 0.0 {
        t += 1.0;
    }
    if t > 1.0 {
        t -= 1.0;
    }
    if t < 1.0 / 6.0 {
        p + (q - p) * 6.0 * t
    } else if t < 0.5 {
        q
    } else if t < 2.0 / 3.0 {
        p + (q - p) * (2.0 / 3.0 - t) * 6.0
    } else {
        p
    }
}

fn shade(rgb: u32, dx: i32, dz: i32) -> u32 {
    let slope = ((dx + dz) / 32).clamp(-24, 24);
    let adjust = |channel: u32| -> u32 {
        (i32::try_from(channel).unwrap_or(0) - slope).clamp(0, 255) as u32
    };
    adjust((rgb >> 16) & 0xff) << 16 | adjust((rgb >> 8) & 0xff) << 8 | adjust(rgb & 0xff)
}

#[derive(Default)]
struct MemoryTracker {
    peak: u64,
}

impl MemoryTracker {
    fn observe(&mut self, bytes: u64) {
        self.peak = self.peak.max(bytes);
    }
}

#[cfg(test)]
#[path = "raster_tests.rs"]
mod tests;
