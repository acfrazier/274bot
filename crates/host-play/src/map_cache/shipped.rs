//! Map data shipped with a release package: terrain imagery and the map
//! catalogue.
//!
//! Release packaging (`tools/release/package.py`) bakes the pinned client
//! cache through the production map path and ships the published
//! directories beside the navigation resources, under the install resource
//! root:
//!
//! ```text
//! map/<revision>/274bot.mapimages.json    identity, key, manifest receipt, totals
//! map/<revision>/images/<key>/…           the published image directory, byte for byte
//! map/<revision>/274bot.mapcatalogue.json identity, key, manifest receipt, totals
//! map/<revision>/catalogues/<key>/…       the published catalogue directory, byte for byte
//! ```
//!
//! The map cache installs each artifact only for exactly the bound client
//! cache's identity for that artifact (revision, decoded content, bake or
//! derive policy); shipped data for any other identity stays on disk,
//! unused. Installing terrain copies each tile into the key's partial
//! directory after checking its length and SHA-256 against the shipped
//! image manifest (itself pinned by the description's receipt), records it
//! by that receipt and publishes through the same writer and atomic rename
//! as a local bake, so readers open it like any other ready terrain.
//! Installing the catalogue checks the shipped manifest against its
//! description receipt and the payload against the manifest's receipt and
//! record count, then hands the decoded catalogue to the same writer and
//! publication as a local derive. A published key is never replaced.
//! Missing, damaged or disagreeing shipped files fail closed: the key is
//! remembered for the process, and the local bake stays available (the
//! catalogue derive runs at once; terrain still waits for consent, with the
//! tiles verified so far checkpointed in the partial directory for a
//! consented bake to adopt). Shipped files are only ever read.

use super::{
    atomic_write, read_bounded, BakePlan, BakeWriter, MapBakeCursor, MapBakeCursorOutput,
    MapBakeCursorStep, MapCacheError, MapCacheRoot, MapProgressSnapshot, MapStage,
};
use api::work::{Checkpoint, MapBakePhase, StepContext};
use nav_planner::map::cache::{BakeStage, CompletedUnit, UnitKey};
use nav_planner::map::formats::{
    CatalogueManifest, ClientPois, ImageManifest, ShippedCatalogue, ShippedImages, TileReceipt,
    MAX_JSON_BYTES, MAX_PNG_BYTES, MAX_SHIPPED_CATALOGUE_BYTES, MAX_SHIPPED_IMAGES_BYTES,
};
use nav_planner::map::identity::{CatalogueIdentity, Digest, ImageIdentity};
use nav_planner::map::MapError;
use parking_lot::Mutex;
use serde::Serialize;
use std::collections::BTreeSet;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Name of the shipped terrain description under `map/<revision>/`.
pub const SHIPPED_IMAGES_NAME: &str = "274bot.mapimages.json";
/// Name of the shipped catalogue description under `map/<revision>/`.
pub const SHIPPED_CATALOGUE_NAME: &str = "274bot.mapcatalogue.json";

/// Both descriptions one shipped map bundle carries, as release packaging
/// prints and records them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ShippedMapBundle {
    pub images: ShippedImages,
    pub catalogue: ShippedCatalogue,
}

/// Map data shipped under an install resource root's `map` directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShippedMap(PathBuf);

impl ShippedMap {
    /// `dir` is the `map` directory beside the navigation resources.
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self(dir.into())
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    /// `map/<revision>/274bot.mapimages.json`.
    pub fn images_description_path(&self, revision: u16) -> PathBuf {
        self.0.join(revision.to_string()).join(SHIPPED_IMAGES_NAME)
    }

    /// `map/<revision>/274bot.mapcatalogue.json`.
    pub fn catalogue_description_path(&self, revision: u16) -> PathBuf {
        self.0
            .join(revision.to_string())
            .join(SHIPPED_CATALOGUE_NAME)
    }

    /// Shipped image directory for `identity`.
    pub fn image_dir(&self, identity: ImageIdentity) -> Result<PathBuf, MapCacheError> {
        Ok(self
            .0
            .join(identity.revision.to_string())
            .join("images")
            .join(identity.key()?.0.to_string()))
    }

    /// Shipped catalogue directory for `identity`.
    pub fn catalogue_dir(&self, identity: CatalogueIdentity) -> Result<PathBuf, MapCacheError> {
        Ok(self
            .0
            .join(identity.revision.to_string())
            .join("catalogues")
            .join(identity.key()?.0.to_string()))
    }

    /// The terrain shipped for exactly `identity`. `None` when nothing is
    /// shipped for its revision, or the shipped terrain belongs to another
    /// client cache or bake policy (it stays on disk, unused).
    pub fn find_images(
        &self,
        identity: ImageIdentity,
    ) -> Result<Option<ShippedImages>, MapCacheError> {
        let bytes = match read_bounded(
            &self.images_description_path(identity.revision),
            MAX_SHIPPED_IMAGES_BYTES,
        ) {
            Ok(bytes) => bytes,
            Err(MapCacheError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(None)
            }
            Err(error) => return Err(error),
        };
        match ShippedImages::decode(&bytes, identity) {
            Ok(shipped) => Ok(Some(shipped)),
            Err(MapError::Identity) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// The catalogue shipped for exactly `identity`. `None` when nothing is
    /// shipped for its revision, or the shipped catalogue belongs to another
    /// client cache or derive policy (it stays on disk, unused).
    pub fn find_catalogue(
        &self,
        identity: CatalogueIdentity,
    ) -> Result<Option<ShippedCatalogue>, MapCacheError> {
        let bytes = match read_bounded(
            &self.catalogue_description_path(identity.revision),
            MAX_SHIPPED_CATALOGUE_BYTES,
        ) {
            Ok(bytes) => bytes,
            Err(MapCacheError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(None)
            }
            Err(error) => return Err(error),
        };
        match ShippedCatalogue::decode(&bytes, identity) {
            Ok(shipped) => Ok(Some(shipped)),
            Err(MapError::Identity) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Release packaging: ship the terrain `root` published for `identity`.
    /// Every tile is checked against its receipt as it is copied, and the
    /// description is written last. Existing shipped terrain is never
    /// replaced.
    pub fn ship_images_from(
        &self,
        root: &MapCacheRoot,
        identity: ImageIdentity,
    ) -> Result<ShippedImages, MapCacheError> {
        // The shared lease keeps a pruner away while the directory is copied.
        let mut capacity = root.try_acquire_capacity()?.ok_or(MapCacheError::Busy)?;
        let _ready = root
            .open_images(identity, &mut capacity)?
            .ok_or(MapCacheError::TerrainNotBaked)?;
        drop(capacity);
        let source = root.image_dir(identity)?;
        let description = self.images_description_path(identity.revision);
        let target = self.image_dir(identity)?;
        if description.exists() || target.exists() {
            return Err(MapCacheError::Message(format!(
                "shipped map terrain already exists under {}",
                self.0.display()
            )));
        }
        let bytes = read_bounded(&source.join("manifest.json"), MAX_JSON_BYTES)?;
        let manifest = ImageManifest::decode(&bytes, identity)?;
        let shipped = ShippedImages::describe(&manifest, &bytes)?;
        let mut buffer = Vec::new();
        for tile in manifest.tiles.as_slice() {
            let relative = tile.key.relative_path()?;
            read_tile(&source.join(&relative), tile, &mut buffer)?;
            atomic_write(&target.join(&relative), &buffer)?;
        }
        atomic_write(&target.join("manifest.json"), &bytes)?;
        atomic_write(&description, &shipped.encode()?)?;
        Ok(shipped)
    }

    /// Release packaging: ship the catalogue `root` published for
    /// `identity`. The payload is checked against its manifest receipt as
    /// it is copied, and the description is written last. An existing
    /// shipped catalogue is never replaced.
    pub fn ship_catalogue_from(
        &self,
        root: &MapCacheRoot,
        identity: CatalogueIdentity,
    ) -> Result<ShippedCatalogue, MapCacheError> {
        // The shared lease keeps a pruner away while the directory is copied.
        let mut capacity = root.try_acquire_capacity()?.ok_or(MapCacheError::Busy)?;
        let _ready = root
            .open_catalogue(identity, &mut capacity)?
            .ok_or_else(|| MapCacheError::Message("map catalogue is not baked".into()))?;
        drop(capacity);
        let source = root.catalogue_dir(identity)?;
        let description = self.catalogue_description_path(identity.revision);
        let target = self.catalogue_dir(identity)?;
        if description.exists() || target.exists() {
            return Err(MapCacheError::Message(format!(
                "shipped map catalogue already exists under {}",
                self.0.display()
            )));
        }
        let bytes = read_bounded(&source.join("manifest.json"), MAX_JSON_BYTES)?;
        let manifest = CatalogueManifest::decode(&bytes, identity)?;
        let shipped = ShippedCatalogue::describe(&manifest, &bytes)?;
        let payload = read_bounded(&source.join("client-pois.json"), MAX_JSON_BYTES)?;
        manifest.read_payload(&payload)?;
        atomic_write(&target.join("client-pois.json"), &payload)?;
        atomic_write(&target.join("manifest.json"), &bytes)?;
        atomic_write(&description, &shipped.encode()?)?;
        Ok(shipped)
    }
}

/// A manager's shipped map data, plus the keys whose shipped data proved
/// unusable in this process (no second attempt, one log line).
pub(super) struct ShippedSource {
    map: ShippedMap,
    rejected: Mutex<BTreeSet<Digest>>,
}

impl ShippedSource {
    pub(super) fn new(map: ShippedMap) -> Self {
        Self {
            map,
            rejected: Mutex::new(BTreeSet::new()),
        }
    }

    /// Shipped terrain for exactly `identity`, unless it already proved
    /// unusable in this process.
    pub(super) fn images_bundle(&self, identity: ImageIdentity) -> Option<ShippedImages> {
        let key = identity.key().ok()?.0;
        if self.rejected.lock().contains(&key) {
            return None;
        }
        match self.map.find_images(identity) {
            Ok(found) => found,
            Err(error) => {
                self.reject(key, "terrain", &error);
                None
            }
        }
    }

    /// Shipped catalogue for exactly `identity`, unless it already proved
    /// unusable in this process.
    pub(super) fn catalogue_bundle(&self, identity: CatalogueIdentity) -> Option<ShippedCatalogue> {
        let key = identity.key().ok()?.0;
        if self.rejected.lock().contains(&key) {
            return None;
        }
        match self.map.find_catalogue(identity) {
            Ok(found) => found,
            Err(error) => {
                self.reject(key, "catalogue", &error);
                None
            }
        }
    }

    pub(super) fn images_cursor(
        &self,
        identity: ImageIdentity,
        bundle: ShippedImages,
        partial_dir: Arc<PathBuf>,
    ) -> Result<Box<dyn MapBakeCursor>, MapCacheError> {
        Ok(Box::new(ShippedInstallCursor {
            source: self.map.image_dir(identity)?,
            identity,
            bundle,
            partial_dir,
            plan: None,
            manifest: None,
            index: 0,
            buffer: Vec::new(),
            completed: Vec::new(),
        }))
    }

    pub(super) fn catalogue_cursor(
        &self,
        identity: CatalogueIdentity,
        bundle: ShippedCatalogue,
    ) -> Result<Box<dyn MapBakeCursor>, MapCacheError> {
        Ok(Box::new(ShippedCatalogueInstallCursor {
            source: self.map.catalogue_dir(identity)?,
            identity,
            bundle,
            output: None,
        }))
    }

    pub(super) fn reject(&self, key: Digest, what: &str, error: &MapCacheError) {
        if self.rejected.lock().insert(key) {
            eprintln!(
                "host-play: shipped map {what} {key} is not used ({error}); \
                 the local bake stays available"
            );
        }
    }
}

struct ShippedInstallCursor {
    source: PathBuf,
    identity: ImageIdentity,
    bundle: ShippedImages,
    partial_dir: Arc<PathBuf>,
    plan: Option<BakePlan>,
    manifest: Option<ImageManifest>,
    index: usize,
    buffer: Vec<u8>,
    completed: Vec<CompletedUnit>,
}

impl MapBakeCursor for ShippedInstallCursor {
    fn plan(&self) -> Option<BakePlan> {
        self.plan
    }

    fn step(
        &mut self,
        context: &mut StepContext<'_, MapBakePhase>,
        writer: Option<&mut BakeWriter>,
    ) -> Result<MapBakeCursorStep, MapCacheError> {
        if self.manifest.is_none() {
            match context.checkpoint() {
                Checkpoint::Continue => {}
                Checkpoint::Yield => return Ok(MapBakeCursorStep::Yielded),
                Checkpoint::Cancel(_) => return Ok(MapBakeCursorStep::Cancelled),
            }
            let bytes = match read_bounded(&self.source.join("manifest.json"), MAX_JSON_BYTES) {
                Ok(bytes) => bytes,
                Err(error) => return Ok(MapBakeCursorStep::Rejected(error)),
            };
            let manifest = match self.bundle.read_manifest(&bytes) {
                Ok(manifest) if manifest.identity == self.identity => manifest,
                Ok(_) => return Ok(MapBakeCursorStep::Rejected(MapError::Identity.into())),
                Err(error) => return Ok(MapBakeCursorStep::Rejected(error.into())),
            };
            let plan = BakePlan {
                planned_units: self.bundle.tiles,
                stage: nav_planner::map::cache::BakeStage::BaseTerrain,
            }
            .checked()?;
            self.plan = Some(plan);
            self.manifest = Some(manifest);
            return Ok(match context.consume_unit() {
                Checkpoint::Cancel(_) => MapBakeCursorStep::Cancelled,
                Checkpoint::Continue | Checkpoint::Yield => MapBakeCursorStep::Yielded,
            });
        }

        let manifest = self
            .manifest
            .as_ref()
            .ok_or(MapCacheError::Map(MapError::Invalid(
                "shipped manifest absent",
            )))?;
        let tiles = manifest.tiles.as_slice();
        let Some(writer) = writer else {
            return Ok(MapBakeCursorStep::Yielded);
        };
        loop {
            match context.checkpoint() {
                Checkpoint::Continue => {}
                Checkpoint::Yield => return Ok(MapBakeCursorStep::Yielded),
                Checkpoint::Cancel(_) => return Ok(MapBakeCursorStep::Cancelled),
            }
            let Some(tile) = tiles.get(self.index) else {
                return Ok(MapBakeCursorStep::Completed(MapBakeCursorOutput::Images(
                    manifest.clone(),
                )));
            };
            if writer.directory() != self.partial_dir.as_path() {
                return Err(MapCacheError::Map(MapError::Path));
            }
            let unit = UnitKey::Terrain { tile: tile.key };
            if let Some(receipt) = writer.completed_receipt(unit) {
                if receipt != tile.payload {
                    return Ok(MapBakeCursorStep::Rejected(
                        MapError::Invalid("checkpointed tile differs from shipped tile").into(),
                    ));
                }
            } else {
                let relative = tile.key.relative_path().map_err(MapCacheError::from)?;
                if let Err(error) = read_tile(&self.source.join(&relative), tile, &mut self.buffer)
                {
                    return Ok(MapBakeCursorStep::Rejected(error));
                }
                let receipt = writer.write_image_tile(unit, &self.buffer)?;
                if receipt != tile.payload {
                    return Ok(MapBakeCursorStep::Rejected(
                        MapError::Invalid("installed tile differs from shipped tile").into(),
                    ));
                }
                self.completed.push(CompletedUnit {
                    key: unit,
                    payload: receipt,
                });
            }
            self.index += 1;
            match context.consume_unit() {
                Checkpoint::Continue => {}
                Checkpoint::Yield => return Ok(MapBakeCursorStep::Yielded),
                Checkpoint::Cancel(_) => return Ok(MapBakeCursorStep::Cancelled),
            }
        }
    }

    fn progress(&self) -> MapProgressSnapshot {
        MapProgressSnapshot {
            stage: MapStage::Publishing,
            completed: u32::try_from(self.index).unwrap_or(u32::MAX),
            total: Some(self.bundle.tiles),
        }
    }

    fn take_completed_units(&mut self) -> Vec<CompletedUnit> {
        std::mem::take(&mut self.completed)
    }
}

struct ShippedCatalogueInstallCursor {
    source: PathBuf,
    identity: CatalogueIdentity,
    bundle: ShippedCatalogue,
    output: Option<ClientPois>,
}

impl ShippedCatalogueInstallCursor {
    /// The shipped catalogue, fully checked: the manifest against the
    /// description's receipt, then the payload against the manifest's
    /// receipt, identity and record count.
    fn read(&self) -> Result<ClientPois, MapCacheError> {
        let bytes = read_bounded(&self.source.join("manifest.json"), MAX_JSON_BYTES)?;
        let manifest = self.bundle.read_manifest(&bytes)?;
        if manifest.identity != self.identity {
            return Err(MapError::Identity.into());
        }
        let payload = read_bounded(
            &self.source.join("client-pois.json"),
            manifest.payload.bytes as usize,
        )?;
        Ok(manifest.read_payload(&payload)?)
    }
}

impl MapBakeCursor for ShippedCatalogueInstallCursor {
    fn plan(&self) -> Option<BakePlan> {
        // Reading the shipped files does not touch cache output. Hand the
        // writer to the host only after the owned result is ready for
        // persistence, like the native catalogue cursor.
        self.output.as_ref().map(|_| BakePlan {
            planned_units: 1,
            stage: BakeStage::Catalogue,
        })
    }

    fn step(
        &mut self,
        context: &mut StepContext<'_, MapBakePhase>,
        _writer: Option<&mut BakeWriter>,
    ) -> Result<MapBakeCursorStep, MapCacheError> {
        if let Some(pois) = self.output.take() {
            return Ok(MapBakeCursorStep::Completed(
                MapBakeCursorOutput::Catalogue(pois),
            ));
        }
        match context.checkpoint() {
            Checkpoint::Continue => {}
            Checkpoint::Yield => return Ok(MapBakeCursorStep::Yielded),
            Checkpoint::Cancel(_) => return Ok(MapBakeCursorStep::Cancelled),
        }
        match self.read() {
            Ok(pois) => self.output = Some(pois),
            Err(error) => return Ok(MapBakeCursorStep::Rejected(error)),
        }
        Ok(match context.consume_unit() {
            Checkpoint::Cancel(_) => MapBakeCursorStep::Cancelled,
            Checkpoint::Continue | Checkpoint::Yield => MapBakeCursorStep::Yielded,
        })
    }

    fn progress(&self) -> MapProgressSnapshot {
        MapProgressSnapshot {
            stage: MapStage::Publishing,
            completed: u32::from(self.output.is_some()),
            total: Some(1),
        }
    }

    fn take_completed_units(&mut self) -> Vec<CompletedUnit> {
        Vec::new()
    }
}

/// Read one tile into `buffer` (reused) and check its length, SHA-256 and
/// PNG header against `tile`'s receipt.
fn read_tile(path: &Path, tile: &TileReceipt, buffer: &mut Vec<u8>) -> Result<(), MapCacheError> {
    let file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(MapError::Path.into());
    }
    if metadata.len() > u64::from(MAX_PNG_BYTES) {
        return Err(MapError::Limit("tile bytes").into());
    }
    buffer.clear();
    file.take(u64::from(MAX_PNG_BYTES) + 1)
        .read_to_end(buffer)?;
    tile.verify_png(buffer)?;
    Ok(())
}

#[cfg(test)]
#[path = "shipped_tests.rs"]
mod tests;
