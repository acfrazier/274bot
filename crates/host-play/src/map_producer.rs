//! Production [`MapBakeProducer`]: B's catalogue/terrain helpers over a leased
//! [`BakeRequest::client_input`].

use crate::map_cache::{
    ArtifactKind, BakeOutput, BakePlan, BakeRequest, BakeWriter, MapBakeProducer, MapCacheError,
    MapStage,
};
use nav::map::cache::{BakeStage, UnitKey};
use nav::map::formats::{CatalogueManifest, PayloadReceipt};
use nav::map::identity::{Digest, CATALOGUE_SCHEMA};
use nav::map::producer::{self, derive_catalogue};
use nav::map::raster::{self, bake_images_into, ImageBakeOutcome, RasterStage};
use nav::map::MapError;
use std::sync::OnceLock;

/// Native client-cache producer. One instance is shared by the process-wide
/// demand manager; it holds no cache path of its own.
#[derive(Debug, Default)]
pub struct NativeMapProducer;

impl NativeMapProducer {
    pub fn new() -> Self {
        Self
    }
}

/// Image and catalogue policy digests for [`crate::map_cache::MapProfileDescriptor`].
pub fn map_artifact_policies() -> Result<(Digest, Digest), MapCacheError> {
    static CACHED: OnceLock<(Digest, Digest)> = OnceLock::new();
    if let Some(value) = CACHED.get() {
        return Ok(*value);
    }
    let value = (
        raster::bake_policy().identity()?,
        producer::catalogue_policy().identity(),
    );
    Ok(*CACHED.get_or_init(|| value))
}

impl MapBakeProducer for NativeMapProducer {
    fn plan(&self, request: &BakeRequest) -> Result<BakePlan, MapCacheError> {
        let input = request.client_input()?;
        match request.artifact() {
            ArtifactKind::Catalogue => Ok(BakePlan {
                planned_units: 1,
                stage: BakeStage::Catalogue,
            }),
            ArtifactKind::Images => {
                let (plan, _) = raster::plan_images(input)?;
                let planned_units = u32::try_from(plan.tiles().len())
                    .map_err(|_| MapCacheError::Map(MapError::Limit("image tile count")))?;
                BakePlan {
                    planned_units,
                    stage: BakeStage::BaseTerrain,
                }
                .checked()
            }
        }
    }

    fn run(
        &self,
        request: &BakeRequest,
        writer: &mut BakeWriter,
    ) -> Result<BakeOutput, MapCacheError> {
        let input = request.client_input()?;
        match request.artifact() {
            ArtifactKind::Catalogue => {
                writer.set_stage(BakeStage::Catalogue, "deriving client POIs")?;
                let (pois, _) = derive_catalogue(input)?;
                let payload = pois.encode()?;
                writer.publish_unit(UnitKey::ClientPois, &payload)?;
                Ok(BakeOutput::Catalogue(CatalogueManifest {
                    schema: CATALOGUE_SCHEMA,
                    identity: pois.identity,
                    key: pois.identity.key()?,
                    record_count: u32::try_from(pois.records.as_slice().len())
                        .map_err(|_| MapCacheError::Map(MapError::Limit("POI count")))?,
                    payload: PayloadReceipt {
                        bytes: payload.len() as u32,
                        sha256: Digest::of(&payload),
                    },
                }))
            }
            ArtifactKind::Images => run_images(input, writer),
        }
    }
}

fn run_images(
    input: nav::map::producer::ClientMapInput<'_>,
    writer: &mut BakeWriter,
) -> Result<BakeOutput, MapCacheError> {
    let dir = writer.directory().to_path_buf();
    let skip: std::collections::BTreeSet<_> = writer
        .checkpoint()
        .completed
        .as_slice()
        .iter()
        .filter_map(|unit| match unit.key {
            UnitKey::Terrain { tile } => Some(tile),
            UnitKey::ClientPois => None,
        })
        .collect();
    let outcome = bake_images_into(
        input,
        &dir,
        |key| skip.contains(&key),
        |progress| {
            if writer.is_cancelled() {
                return false;
            }
            let (stage, message) = match progress.stage {
                RasterStage::ReadingCache => {
                    (MapStage::ReadingCache, String::from("reading cache"))
                }
                RasterStage::BaseTerrain { plane } => {
                    (MapStage::BakingPlane, format!("baking plane {plane}"))
                }
                RasterStage::Downsample { plane, lod } => (
                    MapStage::BuildingZoomLevels,
                    format!("building zoom {lod} plane {plane}"),
                ),
                RasterStage::Publishing => (MapStage::Publishing, String::from("publishing")),
            };
            writer.report_progress(
                stage,
                progress.completed_tiles,
                progress.total_tiles,
                format!(
                    "{message} · {}/{}",
                    progress.completed_tiles, progress.total_tiles
                ),
            );
            true
        },
    )?;
    finish_image_bake(writer, outcome)
}

pub(crate) fn finish_image_bake(
    writer: &mut BakeWriter,
    outcome: ImageBakeOutcome,
) -> Result<BakeOutput, MapCacheError> {
    match outcome {
        ImageBakeOutcome::Complete(report) => {
            // The raster already wrote, synced and hashed every tile in the
            // partial directory (or adopted a verified one from disk), and
            // its manifest lists them in key order. Checkpoint those receipts
            // without reading or rewriting a single PNG.
            for tile in report.manifest.tiles.as_slice() {
                writer.record_unit(UnitKey::Terrain { tile: tile.key }, tile.payload)?;
            }
            Ok(BakeOutput::Images(report.manifest))
        }
        // Nothing is checkpointed on a pause: the next bake adopts every
        // valid tile already on disk (`bake_images_into`).
        ImageBakeOutcome::Paused(_) => Err(MapCacheError::Cancelled),
    }
}
