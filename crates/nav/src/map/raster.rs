//! Deterministic sparse terrain raster and PNG pyramid. Base units decode only
//! a bounded mapsquare neighborhood; coarser units decode at most sixteen
//! child tiles and never construct a dense plane or client `World`.

use super::cache::{ArtifactIdentity, BakeStage, Checkpoint, CompletedUnit, PartialEntry, UnitKey};
use super::client_cache::{
    load_definitions, load_raster_assets, CacheReader, Definitions, FloorDefinition, GroundCell,
    LandSquare, RasterAssets, RasterLocDefinition,
};
use super::formats::{
    ColorFormat, ImageManifest, PayloadReceipt, PlaneBounds, TileReceipt, MAX_IMAGE_BYTES,
    MAX_IMAGE_TILES, MAX_PNG_BYTES,
};
use super::identity::{BakePolicy, Digest, EncoderLibrary, ImageIdentity, IMAGE_SCHEMA};
use super::producer::{source_digest, ClientMapInput};
use super::spatial::{
    TileKey, WorldBounds, MAX_LOD, TILE_GUTTER, TILE_INTERIOR, TILE_PIXELS, TILE_RGBA_BYTES,
};
use super::{MapError, Rows, Text};
use client::graphics::Pix8;
use client::map_cache::{LocPlacement, MapIndexEntry, MAP_SQUARE_SIZE};
use png::{BitDepth, ColorType, Compression, Decoder, Encoder, Filter};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{Cursor, Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};

const FIXED_WALL_RGB: u32 = 0xeeeeee;
const FIXED_ACTIVE_WALL_RGB: u32 = 0xee0000;
const GRID_MARGIN: i32 = 5;

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
    tiles: BTreeSet<TileKey>,
    base_tiles: BTreeSet<TileKey>,
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

#[derive(Debug, Clone)]
pub enum ImageBakeOutcome {
    Complete(ImageBakeReport),
    Paused(RasterMetrics),
}

pub fn bake_policy() -> BakePolicy {
    BakePolicy {
        algorithm: Text::new("native-sparse-terrain-v1").expect("static policy text"),
        producer_sources: source_digest(&[
            include_bytes!("raster.rs"),
            include_bytes!("client_cache.rs"),
            include_bytes!("records.rs"),
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
        ]),
        encoder: EncoderLibrary {
            name: Text::new("png").expect("static encoder name"),
            version: Text::new("0.18.1").expect("static encoder version"),
        },
        libraries: Rows::new(vec![
            EncoderLibrary {
                name: Text::new("crc32fast").expect("static library name"),
                version: Text::new("1.5.0").expect("static library version"),
            },
            EncoderLibrary {
                name: Text::new("fdeflate").expect("static library name"),
                version: Text::new("0.3.7").expect("static library version"),
            },
            EncoderLibrary {
                name: Text::new("flate2").expect("static library name"),
                version: Text::new("1.1.9").expect("static library version"),
            },
            EncoderLibrary {
                name: Text::new("miniz_oxide").expect("static library name"),
                version: Text::new("0.8.9").expect("static library version"),
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
    let identity = image_identity(input)?;
    let mut context = RasterContext::open(input)?;
    context.plan(identity)
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
    if !keep_running(RasterProgress {
        stage: RasterStage::ReadingCache,
        completed_tiles: 0,
        total_tiles: 0,
    }) {
        return Ok(ImageBakeOutcome::Paused(metrics_from(
            &[],
            PlanStats::default(),
            0,
            started.elapsed(),
        )));
    }
    fs::create_dir_all(partial.directory())?;
    let mut context = RasterContext::open(input)?;
    let (plan, plan_stats) = context.plan(identity)?;
    let total_tiles = u32::try_from(plan.tiles.len()).map_err(|_| MapError::Limit("tile count"))?;
    let checkpoint_path = partial.directory().join("checkpoint.json");
    let mut completed = if checkpoint_path.exists() {
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
    if completed.iter().any(|unit| match unit.key {
        UnitKey::Terrain { tile } => !plan.tiles.contains(&tile),
        UnitKey::ClientPois => true,
    }) {
        return Err(MapError::Invalid("checkpoint unit plan"));
    }
    let mut completed_keys: BTreeSet<TileKey> = completed
        .iter()
        .filter_map(|unit| match unit.key {
            UnitKey::Terrain { tile } => Some(tile),
            UnitKey::ClientPois => None,
        })
        .collect();
    let Some(mut completed_bytes) = completed.iter().try_fold(0u64, |total, unit| {
        total.checked_add(u64::from(unit.payload.bytes))
    }) else {
        return Err(MapError::Limit("image bytes"));
    };
    if completed_bytes > MAX_IMAGE_BYTES {
        return Err(MapError::Limit("image bytes"));
    }
    let mut tracker = MemoryTracker::default();
    tracker.observe(context.tracked_bytes());

    let mut groups: BTreeMap<(i32, i32), Vec<u8>> = BTreeMap::new();
    for tile in &plan.base_tiles {
        groups.entry((tile.x, tile.z)).or_default().push(tile.plane);
    }
    for ((square_x, square_z), planes) in groups {
        let Some(next_plane) = planes.iter().copied().find(|plane| {
            !completed_keys.contains(&TileKey {
                plane: *plane,
                lod: 0,
                x: square_x,
                z: square_z,
            })
        }) else {
            continue;
        };
        let progress = RasterProgress {
            stage: RasterStage::BaseTerrain { plane: next_plane },
            completed_tiles: completed_keys.len() as u32,
            total_tiles,
        };
        if !keep_running(progress) {
            return Ok(ImageBakeOutcome::Paused(metrics_from(
                &completed,
                plan_stats,
                tracker.peak,
                started.elapsed(),
            )));
        }
        let neighborhood = context.load_neighborhood(square_x, square_z)?;
        tracker.observe(context.tracked_bytes() + neighborhood.tracked_bytes());
        for plane in planes {
            let key = TileKey {
                plane,
                lod: 0,
                x: square_x,
                z: square_z,
            };
            if completed_keys.contains(&key) {
                continue;
            }
            let rgba = render_base(&context, &neighborhood, key)?;
            tracker.observe(
                context.tracked_bytes() + neighborhood.tracked_bytes() + rgba.capacity() as u64,
            );
            let png = encode_png(&rgba)?;
            tracker.observe(
                context.tracked_bytes()
                    + neighborhood.tracked_bytes()
                    + rgba.capacity() as u64
                    + png.capacity() as u64,
            );
            complete_tile(
                partial.directory(),
                identity,
                total_tiles,
                BakeStage::BaseTerrain,
                key,
                &png,
                &mut completed,
                &mut completed_keys,
                &mut completed_bytes,
            )?;
        }
    }

    write_checkpoint(
        partial.directory(),
        identity,
        BakeStage::Downsample,
        total_tiles,
        &completed,
    )?;
    let mut compressed = Vec::new();
    for lod in 1..=plan.max_lod {
        for key in plan.tiles.iter().copied().filter(|key| key.lod == lod) {
            if completed_keys.contains(&key) {
                continue;
            }
            let progress = RasterProgress {
                stage: RasterStage::Downsample {
                    plane: key.plane,
                    lod,
                },
                completed_tiles: completed_keys.len() as u32,
                total_tiles,
            };
            if !keep_running(progress) {
                return Ok(ImageBakeOutcome::Paused(metrics_from(
                    &completed,
                    plan_stats,
                    tracker.peak,
                    started.elapsed(),
                )));
            }
            let (rgba, child_bytes) =
                render_parent(partial.directory(), &plan, &completed, key, &mut compressed)?;
            tracker.observe(context.tracked_bytes() + child_bytes + rgba.capacity() as u64);
            let png = encode_png(&rgba)?;
            tracker.observe(
                context.tracked_bytes()
                    + child_bytes
                    + rgba.capacity() as u64
                    + png.capacity() as u64,
            );
            complete_tile(
                partial.directory(),
                identity,
                total_tiles,
                BakeStage::Downsample,
                key,
                &png,
                &mut completed,
                &mut completed_keys,
                &mut completed_bytes,
            )?;
        }
    }

    if completed.len() != plan.tiles.len() {
        return Err(MapError::Invalid("incomplete image plan"));
    }
    let receipts: Vec<TileReceipt> = completed
        .iter()
        .map(|unit| match unit.key {
            UnitKey::Terrain { tile } => Ok(TileReceipt {
                key: tile,
                payload: unit.payload,
            }),
            UnitKey::ClientPois => Err(MapError::Invalid("image checkpoint unit")),
        })
        .collect::<Result<_, _>>()?;
    let manifest = ImageManifest {
        schema: IMAGE_SCHEMA,
        identity,
        key: identity.key()?,
        extent: plan.extent,
        planes: Rows::new(plan.planes.clone())?,
        max_lod: plan.max_lod,
        interior: TILE_INTERIOR,
        gutter: TILE_GUTTER,
        color: ColorFormat::Rgba8Unorm,
        tiles: Rows::new(receipts)?,
    };
    let manifest_bytes = manifest.encode()?;
    atomic_write(&partial.directory().join("manifest.json"), &manifest_bytes)?;
    write_checkpoint(
        partial.directory(),
        identity,
        BakeStage::Publishing,
        total_tiles,
        &completed,
    )?;
    let progress = RasterProgress {
        stage: RasterStage::Publishing,
        completed_tiles: total_tiles,
        total_tiles,
    };
    let _ = keep_running(progress);
    Ok(ImageBakeOutcome::Complete(ImageBakeReport {
        manifest,
        metrics: metrics_from(&completed, plan_stats, tracker.peak, started.elapsed()),
    }))
}

struct RasterContext {
    definitions: Definitions,
    assets: RasterAssets,
    reader: CacheReader,
}

impl RasterContext {
    fn open(input: ClientMapInput<'_>) -> Result<Self, MapError> {
        Ok(Self {
            definitions: load_definitions(input)?,
            assets: load_raster_assets(input)?,
            reader: CacheReader::open(input)?,
        })
    }

    fn plan(&mut self, identity: ImageIdentity) -> Result<(ImagePlan, PlanStats), MapError> {
        let extent = self.reader.extent()?;
        let entries = self.reader.entries.clone();
        let mut base_tiles = BTreeSet::new();
        let mut stats = PlanStats {
            map_squares: entries.len() as u32,
            ..PlanStats::default()
        };
        for entry in entries {
            let land = self.reader.read_land(entry)?;
            let mut present = [false; 4];
            for x in 0..MAP_SQUARE_SIZE {
                for z in 0..MAP_SQUARE_SIZE {
                    let link_below = land.link_below(x, z);
                    for raw_plane in 0..4u8 {
                        let cell = land.cell(raw_plane, x, z);
                        if cell.underlay == 0 && cell.overlay == 0 {
                            continue;
                        }
                        if let Some(plane) =
                            crate::collision::game_plane(i32::from(raw_plane), link_below)
                        {
                            present[plane as usize] = true;
                        }
                    }
                }
            }
            let mut placement_error = None;
            self.reader.visit_locs(entry, |placement| {
                stats.loc_placements += 1;
                if placement_error.is_some() {
                    return;
                }
                let Some(definition) = self.definitions.locs.get(placement.id as usize) else {
                    placement_error = Some(MapError::Invalid("location definition id"));
                    return;
                };
                let Some(plane) = crate::collision::game_plane(
                    i32::from(placement.plane),
                    land.link_below(placement.x, placement.z),
                ) else {
                    return;
                };
                if (definition.mapscene.is_some() && !matches!(placement.shape, 4..=8))
                    || draws_native_detail(placement.shape)
                {
                    present[plane as usize] = true;
                }
                if let Err(error) = mark_mapscene_tiles(
                    &mut base_tiles,
                    extent,
                    entry,
                    placement,
                    definition,
                    &self.assets,
                    plane as u8,
                ) {
                    placement_error = Some(error);
                }
            })?;
            if let Some(error) = placement_error {
                return Err(error);
            }
            for (plane, present) in present.into_iter().enumerate() {
                if present {
                    base_tiles.insert(TileKey {
                        plane: plane as u8,
                        lod: 0,
                        x: i32::from(entry.square_x),
                        z: i32::from(entry.square_z),
                    });
                }
            }
        }
        if base_tiles.is_empty() {
            return Err(MapError::Invalid("empty client imagery"));
        }
        let mut tiles = base_tiles.clone();
        let mut level = base_tiles.clone();
        let mut max_lod = 0u8;
        loop {
            let represented: BTreeSet<u8> = level.iter().map(|tile| tile.plane).collect();
            if represented
                .iter()
                .all(|plane| level.iter().filter(|tile| tile.plane == *plane).count() == 1)
            {
                break;
            }
            max_lod = max_lod
                .checked_add(1)
                .ok_or(MapError::Limit("pyramid LOD"))?;
            if max_lod > MAX_LOD {
                return Err(MapError::Limit("pyramid LOD"));
            }
            level = level
                .iter()
                .map(|tile| TileKey {
                    plane: tile.plane,
                    lod: max_lod,
                    x: tile.x.div_euclid(2),
                    z: tile.z.div_euclid(2),
                })
                .collect();
            tiles.extend(level.iter().copied());
            if tiles.len() > MAX_IMAGE_TILES {
                return Err(MapError::Limit("image tile count"));
            }
        }
        let mut planes = Vec::new();
        for plane in 0..4u8 {
            let mut plane_tiles = base_tiles.iter().filter(|tile| tile.plane == plane);
            let Some(first) = plane_tiles.next() else {
                continue;
            };
            let mut west = first.x;
            let mut south = first.z;
            let mut east = first.x + 1;
            let mut north = first.z + 1;
            for tile in plane_tiles {
                west = west.min(tile.x);
                south = south.min(tile.z);
                east = east.max(tile.x + 1);
                north = north.max(tile.z + 1);
            }
            planes.push(PlaneBounds {
                plane,
                bounds: WorldBounds {
                    west: west * 64,
                    south: south * 64,
                    east: east * 64,
                    north: north * 64,
                },
            });
        }
        stats.base_tiles = base_tiles.len() as u32;
        stats.pyramid_tiles = tiles.len() as u32;
        stats.max_record_bytes = self.reader.max_buffer_bytes();
        Ok((
            ImagePlan {
                identity,
                extent,
                planes,
                max_lod,
                tiles,
                base_tiles,
            },
            stats,
        ))
    }

    fn load_neighborhood(
        &mut self,
        square_x: i32,
        square_z: i32,
    ) -> Result<Neighborhood, MapError> {
        let mut squares = BTreeMap::new();
        for x in square_x - 1..=square_x + 1 {
            for z in square_z - 1..=square_z + 1 {
                let Some(entry) = self.reader.entry(x, z) else {
                    continue;
                };
                let land = self.reader.read_land(entry)?;
                let mut locs = Vec::new();
                self.reader
                    .visit_locs(entry, |placement| locs.push(placement))?;
                squares.insert((x, z), SquareData { land, locs });
            }
        }
        Ok(Neighborhood { squares })
    }

    fn tracked_bytes(&self) -> u64 {
        let definitions = self.definitions.locs.capacity()
            * std::mem::size_of::<RasterLocDefinition>()
            + self.definitions.floors.capacity() * std::mem::size_of::<FloorDefinition>();
        let sprites: usize = self.assets.mapscenes.capacity() * std::mem::size_of::<Option<Pix8>>()
            + self
                .assets
                .mapscenes
                .iter()
                .flatten()
                .map(|sprite| sprite.data.capacity() + sprite.bpal.capacity() * 4)
                .sum::<usize>();
        (definitions
            + sprites
            + std::mem::size_of_val(&self.assets.texture_colours)
            + self.reader.tracked_bytes()) as u64
    }
}

struct SquareData {
    land: LandSquare,
    locs: Vec<LocPlacement>,
}

struct Neighborhood {
    squares: BTreeMap<(i32, i32), SquareData>,
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

    fn normalized_cell(&self, plane: u8, world_x: i32, world_z: i32) -> Option<GroundCell> {
        let (square, x, z) = self.square(world_x, world_z)?;
        let raw_plane = plane.checked_add(u8::from(square.land.link_below(x, z)))?;
        if raw_plane >= 4 {
            return None;
        }
        Some(square.land.cell(raw_plane, x, z))
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
) -> Result<Vec<u8>, MapError> {
    let mut rgba = vec![0; TILE_RGBA_BYTES];
    let west = key.x * 64;
    let north = (key.z + 1) * 64;
    let grid_west = west - 1 - GRID_MARGIN;
    let grid_south = key.z * 64 - 1 - GRID_MARGIN;
    let grid_size = 66 + GRID_MARGIN * 2;
    let mut grid = Vec::with_capacity((grid_size * grid_size) as usize);
    for x in 0..grid_size {
        for z in 0..grid_size {
            grid.push(neighborhood.normalized_cell(key.plane, grid_west + x, grid_south + z));
        }
    }
    let grid_cell = |world_x: i32, world_z: i32| -> Option<GroundCell> {
        let x = world_x - grid_west;
        let z = world_z - grid_south;
        if x < 0 || z < 0 || x >= grid_size || z >= grid_size {
            return None;
        }
        grid[(x * grid_size + z) as usize]
    };

    for world_x in west - 1..=west + 64 {
        for world_z in key.z * 64 - 1..=key.z * 64 + 64 {
            let Some(cell) = grid_cell(world_x, world_z) else {
                continue;
            };
            if cell.underlay == 0 && cell.overlay == 0 {
                continue;
            }
            let underlay =
                blended_underlay(&context.definitions, &grid_cell, world_x, world_z, cell);
            let overlay = overlay_colour(&context.definitions, &context.assets, cell.overlay);
            let pixel_x = 1 + (world_x - west) * 4;
            let pixel_y = 1 + (north - world_z - 1) * 4;
            draw_floor_cell(
                &mut rgba,
                pixel_x,
                pixel_y,
                underlay,
                overlay,
                cell.overlay_shape,
                cell.overlay_rotation,
            );
        }
    }

    for square in neighborhood.squares.values() {
        for placement in &square.locs {
            let Some(definition) = context.definitions.locs.get(placement.id as usize) else {
                return Err(MapError::Invalid("location definition id"));
            };
            let link_below = square.land.link_below(placement.x, placement.z);
            if crate::collision::game_plane(i32::from(placement.plane), link_below)
                != Some(i32::from(key.plane))
            {
                continue;
            }
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
    Ok(rgba)
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

#[allow(clippy::too_many_arguments)]
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

fn render_parent(
    directory: &Path,
    plan: &ImagePlan,
    completed: &[CompletedUnit],
    parent: TileKey,
    compressed: &mut Vec<u8>,
) -> Result<(Vec<u8>, u64), MapError> {
    let child_lod = parent.lod - 1;
    let mut children: Vec<Option<Vec<u8>>> = (0..16).map(|_| None).collect();
    let mut child_bytes = 0u64;
    for qx in -1..=2 {
        for qy in -1..=2 {
            let child = TileKey {
                plane: parent.plane,
                lod: child_lod,
                x: parent.x * 2 + qx,
                z: parent.z * 2 + 1 - qy,
            };
            if !plan.tiles.contains(&child) {
                continue;
            }
            let receipt = completed
                .binary_search_by_key(&UnitKey::Terrain { tile: child }, |unit| unit.key)
                .ok()
                .map(|index| completed[index].payload)
                .ok_or(MapError::Invalid("missing completed child"))?;
            read_bounded(
                &directory.join(child.relative_path()?),
                MAX_PNG_BYTES as usize,
                compressed,
            )?;
            receipt.verify(compressed, MAX_PNG_BYTES)?;
            let decoded = decode_png(compressed)?;
            child_bytes += decoded.capacity() as u64;
            children[((qx + 1) * 4 + (qy + 1)) as usize] = Some(decoded);
        }
    }
    child_bytes += (compressed.capacity()
        + children.capacity() * std::mem::size_of::<Option<Vec<u8>>>()) as u64;
    let mut output = vec![0u8; TILE_RGBA_BYTES];
    for y in 0..TILE_PIXELS as i32 {
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
    Ok((output, child_bytes))
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
    let decoder = Decoder::new(Cursor::new(bytes));
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
    let mut output = vec![0; size];
    let frame = reader
        .next_frame(&mut output)
        .map_err(|_| MapError::Invalid("PNG decode data"))?;
    if frame.buffer_size() != TILE_RGBA_BYTES {
        return Err(MapError::Invalid("PNG frame length"));
    }
    Ok(output)
}

#[allow(clippy::too_many_arguments)]
fn complete_tile(
    directory: &Path,
    identity: ImageIdentity,
    total_tiles: u32,
    stage: BakeStage,
    key: TileKey,
    png: &[u8],
    completed: &mut Vec<CompletedUnit>,
    completed_keys: &mut BTreeSet<TileKey>,
    completed_bytes: &mut u64,
) -> Result<(), MapError> {
    let payload = PayloadReceipt {
        bytes: png.len() as u32,
        sha256: Digest::of(png),
    };
    let receipt = TileReceipt { key, payload };
    let Some(projected_bytes) = completed_bytes.checked_add(u64::from(payload.bytes)) else {
        return Err(MapError::Limit("image bytes"));
    };
    if projected_bytes > MAX_IMAGE_BYTES {
        return Err(MapError::Limit("image bytes"));
    }
    receipt.verify_png(png)?;
    atomic_write(&directory.join(key.relative_path()?), png)?;
    completed.push(CompletedUnit {
        key: UnitKey::Terrain { tile: key },
        payload,
    });
    completed.sort_unstable_by_key(|unit| unit.key);
    if !completed_keys.insert(key) {
        return Err(MapError::Duplicate("completed tile"));
    }
    *completed_bytes = projected_bytes;
    write_checkpoint(directory, identity, stage, total_tiles, completed)
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
