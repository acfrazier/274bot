//! Production [`MapBakeProducer`]: native resumable map cursors over a leased
//! [`BakeRequest::client_input`].

use crate::map_cache::{
    ArtifactKind, BakePlan, BakeRequest, BakeWriter, MapBakeCursor, MapBakeCursorOutput,
    MapBakeCursorStep, MapBakeProducer, MapCacheError, MapProgressSnapshot, MapStage,
};
use nav::map::cache::{BakeStage, CompletedUnit, UnitKey};
use nav::map::formats::{ClientPois, PayloadReceipt};
use nav::map::identity::Digest;
use nav::map::producer::{self, CatalogueCursor, CatalogueProgress, CatalogueStage};
use nav::map::raster::{
    self, ImageBakeCursor, ImageBakeError, ImageTileSink, RasterProgress, RasterStage,
};
use nav::map::spatial::TileKey;
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
    fn charge(&self, request: &BakeRequest) -> Result<api::work::WorkCharge, MapCacheError> {
        // CatalogueCursor::work_charge always includes the cold game_data
        // initializer peak; the immutable process-static owner is measured
        // separately by the release harness.
        let input = request.client_input()?;
        // Each `work_charge` counts its own cursor. The boxed wrapper adds
        // only the fields it holds beside that cursor.
        let (charge, wrapper_bytes) = match request.artifact() {
            ArtifactKind::Catalogue => (
                CatalogueCursor::work_charge(input).map_err(MapCacheError::Map)?,
                std::mem::size_of::<NativeCatalogueCursor>()
                    - std::mem::size_of::<CatalogueCursor>(),
            ),
            ArtifactKind::Images => (
                ImageBakeCursor::work_charge(input, request.partial_dir_arc())
                    .map_err(MapCacheError::Map)?,
                std::mem::size_of::<NativeImageCursor>() - std::mem::size_of::<ImageBakeCursor>(),
            ),
        };
        let input_bytes =
            charge
                .input_bytes()
                .checked_add(wrapper_bytes)
                .ok_or(MapCacheError::Map(nav::map::MapError::Limit(
                    "native map cursor charge",
                )))?;
        Ok(api::work::WorkCharge::new(
            input_bytes,
            charge.scratch_bytes(),
            charge.result_bytes(),
            charge.operation_handles(),
        ))
    }

    fn cursor(
        &self,
        request: &BakeRequest,
        partial_dir: std::sync::Arc<std::path::PathBuf>,
    ) -> Result<Box<dyn MapBakeCursor>, MapCacheError> {
        let input = request.client_input()?;
        let cursor: Box<dyn MapBakeCursor> = match request.artifact() {
            ArtifactKind::Catalogue => Box::new(NativeCatalogueCursor {
                cursor: CatalogueCursor::new(input).map_err(MapCacheError::Map)?,
                output: None,
            }),
            ArtifactKind::Images => Box::new(NativeImageCursor(
                ImageBakeCursor::new(input, partial_dir, request.descriptor().image_identity())
                    .map_err(MapCacheError::Map)?,
            )),
        };
        Ok(cursor)
    }
}

// The trait-object Box is the only allocation of each cursor, and it holds
// only the cursor type of its artifact.
struct NativeCatalogueCursor {
    cursor: CatalogueCursor,
    output: Option<ClientPois>,
}

struct NativeImageCursor(ImageBakeCursor);

impl MapBakeCursor for NativeCatalogueCursor {
    fn plan(&self) -> Option<BakePlan> {
        // Input work does not touch cache output. Hand the writer to the
        // host only after the owned result is ready for persistence.
        self.output.as_ref().map(|_| BakePlan {
            planned_units: 1,
            stage: BakeStage::Catalogue,
        })
    }

    fn step(
        &mut self,
        context: &mut api::work::StepContext<'_, api::work::MapBakePhase>,
        _writer: Option<&mut BakeWriter>,
    ) -> Result<MapBakeCursorStep, MapCacheError> {
        if let Some(pois) = self.output.take() {
            return Ok(MapBakeCursorStep::Completed(
                MapBakeCursorOutput::Catalogue(pois),
            ));
        }
        let result = self.cursor.step(context).map_err(MapCacheError::Map)?;
        match result {
            producer::ProducerStep::Yielded => Ok(MapBakeCursorStep::Yielded),
            producer::ProducerStep::Completed(result) => {
                self.output = Some(result.pois);
                Ok(MapBakeCursorStep::Yielded)
            }
            producer::ProducerStep::Cancelled => Ok(MapBakeCursorStep::Cancelled),
        }
    }

    fn progress(&self) -> MapProgressSnapshot {
        catalogue_progress(self.cursor.progress())
    }

    fn take_completed_units(&mut self) -> Vec<CompletedUnit> {
        Vec::new()
    }
}

impl MapBakeCursor for NativeImageCursor {
    fn plan(&self) -> Option<BakePlan> {
        self.0.planned_units().map(|planned_units| BakePlan {
            planned_units,
            stage: BakeStage::BaseTerrain,
        })
    }

    fn step(
        &mut self,
        context: &mut api::work::StepContext<'_, api::work::MapBakePhase>,
        writer: Option<&mut BakeWriter>,
    ) -> Result<MapBakeCursorStep, MapCacheError> {
        let cursor = &mut self.0;
        let mut sink = match writer {
            Some(writer) => WriterImageTileSink { writer },
            None => {
                return match cursor
                    .step(context, &mut BackpressureImageTileSink)
                    .map_err(map_image_error)?
                {
                    producer::ProducerStep::Yielded => Ok(MapBakeCursorStep::Yielded),
                    producer::ProducerStep::Completed(report) => Ok(MapBakeCursorStep::Completed(
                        MapBakeCursorOutput::Images(report.manifest),
                    )),
                    producer::ProducerStep::Cancelled => Ok(MapBakeCursorStep::Cancelled),
                }
            }
        };
        match cursor.step(context, &mut sink).map_err(map_image_error)? {
            producer::ProducerStep::Yielded => Ok(MapBakeCursorStep::Yielded),
            producer::ProducerStep::Completed(report) => Ok(MapBakeCursorStep::Completed(
                MapBakeCursorOutput::Images(report.manifest),
            )),
            producer::ProducerStep::Cancelled => Ok(MapBakeCursorStep::Cancelled),
        }
    }

    fn progress(&self) -> MapProgressSnapshot {
        image_progress(self.0.progress())
    }

    fn take_completed_units(&mut self) -> Vec<CompletedUnit> {
        self.0.take_completed_units()
    }
}

struct WriterImageTileSink<'a> {
    writer: &'a mut BakeWriter,
}

impl ImageTileSink for WriterImageTileSink<'_> {
    type Error = MapCacheError;

    fn checkpointed(&mut self, key: TileKey) -> Option<PayloadReceipt> {
        self.writer
            .completed_receipt(UnitKey::Terrain { tile: key })
    }

    fn write_tile(
        &mut self,
        key: TileKey,
        png: &[u8],
    ) -> Result<Option<PayloadReceipt>, Self::Error> {
        self.writer
            .write_image_tile(UnitKey::Terrain { tile: key }, png)
            .map(Some)
    }
}

struct BackpressureImageTileSink;

impl ImageTileSink for BackpressureImageTileSink {
    type Error = MapCacheError;

    fn checkpointed(&mut self, _key: TileKey) -> Option<PayloadReceipt> {
        None
    }

    fn write_tile(
        &mut self,
        _key: TileKey,
        _png: &[u8],
    ) -> Result<Option<PayloadReceipt>, Self::Error> {
        Ok(None)
    }
}

fn map_image_error(error: ImageBakeError<MapCacheError>) -> MapCacheError {
    match error {
        ImageBakeError::Native(error) => MapCacheError::Map(error),
        ImageBakeError::Sink(error) => error,
    }
}

fn catalogue_progress(progress: CatalogueProgress) -> MapProgressSnapshot {
    let (stage, completed, total) = match progress.stage {
        CatalogueStage::Preparing | CatalogueStage::Indexing => (
            MapStage::ReadingCache,
            u32::try_from(progress.units_completed).unwrap_or(u32::MAX),
            None,
        ),
        CatalogueStage::Scanning => (
            MapStage::DerivingPois,
            progress.map_squares_completed,
            progress.map_squares_total,
        ),
        CatalogueStage::Matching => (
            MapStage::DerivingPois,
            progress.map_squares_completed,
            progress.map_squares_total,
        ),
        CatalogueStage::Finalizing => (
            MapStage::DerivingPois,
            progress.poi_records_ready,
            progress.poi_records_total,
        ),
        CatalogueStage::Complete => (
            MapStage::Publishing,
            progress.poi_records_ready,
            progress
                .poi_records_total
                .or(Some(progress.poi_records_ready)),
        ),
    };
    MapProgressSnapshot {
        stage,
        completed,
        total,
    }
}

fn image_progress(progress: RasterProgress) -> MapProgressSnapshot {
    match progress.stage {
        RasterStage::ReadingCache => MapProgressSnapshot {
            stage: MapStage::ReadingCache,
            completed: progress.planning_squares_completed,
            total: progress.planning_squares_total,
        },
        RasterStage::BaseTerrain { .. } => MapProgressSnapshot {
            stage: MapStage::BakingPlane,
            completed: progress.completed_tiles,
            total: (progress.total_tiles != 0).then_some(progress.total_tiles),
        },
        RasterStage::Downsample { .. } => MapProgressSnapshot {
            stage: MapStage::BuildingZoomLevels,
            completed: progress.completed_tiles,
            total: (progress.total_tiles != 0).then_some(progress.total_tiles),
        },
        RasterStage::Publishing => MapProgressSnapshot {
            stage: MapStage::Publishing,
            completed: progress.completed_tiles,
            total: (progress.total_tiles != 0).then_some(progress.total_tiles),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_input_work_does_not_request_a_cache_writer() {
        let input = producer::ClientMapInput::new(
            289,
            Digest([7; 32]),
            std::path::Path::new("/verified/jag"),
            std::path::Path::new("/verified/snapshot"),
        )
        .unwrap();
        let cursor = NativeCatalogueCursor {
            cursor: CatalogueCursor::new(input).unwrap(),
            output: None,
        };
        assert!(
            cursor.plan().is_none(),
            "catalogue input work must not request cache ownership"
        );
    }

    #[test]
    fn image_progress_uses_square_counts_before_plan_and_tile_counts_afterward() {
        let reading = image_progress(RasterProgress {
            stage: RasterStage::ReadingCache,
            completed_tiles: 0,
            total_tiles: 0,
            planning_squares_completed: 12,
            planning_squares_total: None,
        });
        assert_eq!(reading.stage, MapStage::ReadingCache);
        assert_eq!(reading.completed, 12);
        assert_eq!(reading.total, None);

        let planned = image_progress(RasterProgress {
            stage: RasterStage::ReadingCache,
            completed_tiles: 0,
            total_tiles: 0,
            planning_squares_completed: 12,
            planning_squares_total: Some(96),
        });
        assert_eq!(planned.completed, 12);
        assert_eq!(planned.total, Some(96));

        let terrain = image_progress(RasterProgress {
            stage: RasterStage::BaseTerrain { plane: 0 },
            completed_tiles: 7,
            total_tiles: 28,
            planning_squares_completed: 96,
            planning_squares_total: Some(96),
        });
        assert_eq!(terrain.stage, MapStage::BakingPlane);
        assert_eq!(terrain.completed, 7);
        assert_eq!(terrain.total, Some(28));
    }
}
