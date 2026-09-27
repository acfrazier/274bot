//! Terrain imagery shipped with a release package.
//!
//! Release packaging (`tools/release/package.py`) bakes the pinned client
//! cache's terrain through the production image path and ships the published
//! directory beside the navigation resources, under the install resource root:
//!
//! ```text
//! map/<revision>/274bot.mapimages.json  identity, key, manifest receipt, totals
//! map/<revision>/images/<key>/…         the published cache directory, byte for byte
//! ```
//!
//! The map cache installs it only for exactly the bound client cache's image
//! identity (revision, decoded content, bake policy); shipped terrain for any
//! other identity stays on disk, unused. Installing copies each tile into the
//! key's partial directory after checking its length and SHA-256 against the
//! shipped image manifest (itself pinned by the description's receipt),
//! records it by that receipt and publishes through the same writer and
//! atomic rename as a local bake, so readers open it like any other ready
//! terrain. A published key is never replaced. Missing, damaged or
//! disagreeing shipped files fail closed: the key is remembered for the
//! process, the demand settles without terrain (or bakes, with consent), and
//! the tiles verified so far stay checkpointed in the partial directory for a
//! consented bake to adopt. Shipped files are only ever read.

use super::{
    atomic_write, read_bounded, BakeOutput, BakePlan, BakeRequest, BakeWriter, MapCacheError,
    MapCacheRoot, MapProgress, MapStage,
};
use nav::map::cache::{BakeStage, UnitKey};
use nav::map::formats::{
    ImageManifest, ShippedImages, TileReceipt, MAX_JSON_BYTES, MAX_PNG_BYTES,
    MAX_SHIPPED_IMAGES_BYTES,
};
use nav::map::identity::{Digest, ImageIdentity};
use nav::map::MapError;
use parking_lot::Mutex;
use std::collections::BTreeSet;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// Name of the shipped description under `map/<revision>/`.
pub const SHIPPED_IMAGES_NAME: &str = "274bot.mapimages.json";
/// Install progress is reported once per this many tiles.
const REPORT_INTERVAL: usize = 32;

/// Terrain shipped under an install resource root's `map` directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShippedMapImages(PathBuf);

impl ShippedMapImages {
    /// `dir` is the `map` directory beside the navigation resources.
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self(dir.into())
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    /// `map/<revision>/274bot.mapimages.json`.
    pub fn description_path(&self, revision: u16) -> PathBuf {
        self.0.join(revision.to_string()).join(SHIPPED_IMAGES_NAME)
    }

    /// Shipped image directory for `identity`.
    pub fn image_dir(&self, identity: ImageIdentity) -> Result<PathBuf, MapCacheError> {
        Ok(self
            .0
            .join(identity.revision.to_string())
            .join("images")
            .join(identity.key()?.0.to_string()))
    }

    /// The terrain shipped for exactly `identity`. `None` when nothing is
    /// shipped for its revision, or the shipped terrain belongs to another
    /// client cache or bake policy (it stays on disk, unused).
    pub fn find(&self, identity: ImageIdentity) -> Result<Option<ShippedImages>, MapCacheError> {
        let bytes = match read_bounded(
            &self.description_path(identity.revision),
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

    /// Release packaging: ship the terrain `root` published for `identity`.
    /// Every tile is checked against its receipt as it is copied, and the
    /// description is written last. Existing shipped terrain is never
    /// replaced.
    pub fn ship_from(
        &self,
        root: &MapCacheRoot,
        identity: ImageIdentity,
    ) -> Result<ShippedImages, MapCacheError> {
        // The shared lease keeps a pruner away while the directory is copied.
        let _ready = root
            .open_images(identity)?
            .ok_or(MapCacheError::TerrainNotBaked)?;
        let source = root.image_dir(identity)?;
        let description = self.description_path(identity.revision);
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
}

/// A manager's shipped terrain, plus the keys whose shipped terrain proved
/// unusable in this process (no second attempt, one log line).
pub(super) struct ShippedSource {
    images: ShippedMapImages,
    rejected: Mutex<BTreeSet<Digest>>,
}

/// What installing shipped terrain did.
pub(super) enum Installed {
    /// A complete partial directory, ready for the atomic publish.
    Published(PathBuf),
    /// The shipped terrain is unusable (logged and remembered).
    Rejected,
}

impl ShippedSource {
    pub(super) fn new(images: ShippedMapImages) -> Self {
        Self {
            images,
            rejected: Mutex::new(BTreeSet::new()),
        }
    }

    /// Shipped terrain for exactly `identity`, unless it already proved
    /// unusable in this process.
    pub(super) fn bundle(&self, identity: ImageIdentity) -> Option<ShippedImages> {
        let key = identity.key().ok()?.0;
        if self.rejected.lock().contains(&key) {
            return None;
        }
        match self.images.find(identity) {
            Ok(found) => found,
            Err(error) => {
                self.reject(key, &error);
                None
            }
        }
    }

    fn reject(&self, key: Digest, error: &MapCacheError) {
        if self.rejected.lock().insert(key) {
            eprintln!(
                "host-play: shipped map terrain {key} is not used ({error}); \
                 the local bake stays available"
            );
        }
    }

    /// Install `bundle` for `request`'s image identity: verify the shipped
    /// manifest, then copy, verify and record every tile into the key's
    /// partial directory (resuming a matching checkpoint), and finish it for
    /// publication. The caller holds the key's exclusive lock and publishes.
    pub(super) fn install(
        &self,
        bundle: &ShippedImages,
        root: &MapCacheRoot,
        request: &BakeRequest,
        cancel: Arc<AtomicBool>,
        progress: Arc<dyn Fn(MapProgress) + Send + Sync>,
        required: BTreeSet<PathBuf>,
    ) -> Result<Installed, MapCacheError> {
        let identity = request.descriptor().image_identity();
        let key = identity.key()?.0;
        let source = self.images.image_dir(identity)?;
        let manifest = match read_bounded(&source.join("manifest.json"), MAX_JSON_BYTES)
            .and_then(|bytes| Ok(bundle.read_manifest(&bytes)?))
        {
            Ok(manifest) => manifest,
            Err(error) => {
                self.reject(key, &error);
                return Ok(Installed::Rejected);
            }
        };
        let plan = BakePlan {
            planned_units: bundle.tiles,
            stage: BakeStage::BaseTerrain,
        }
        .checked()?;
        let mut writer = BakeWriter::open(root.clone(), request, plan, cancel, progress, required)?;
        writer.set_stage(BakeStage::BaseTerrain, "installing shipped terrain")?;
        let tiles = manifest.tiles.as_slice();
        let mut buffer = Vec::new();
        for (index, tile) in tiles.iter().enumerate() {
            match install_tile(&source, tile, &mut writer, &mut buffer) {
                Ok(()) => {}
                Err(TileFailure::Shipped(error)) => {
                    // Keep the tiles verified so far for a consented bake.
                    writer.persist_checkpoint()?;
                    self.reject(key, &error);
                    return Ok(Installed::Rejected);
                }
                Err(TileFailure::Cache(error)) => {
                    let _ = writer.persist_checkpoint();
                    return Err(error);
                }
            }
            let done = index + 1;
            if done % REPORT_INTERVAL == 0 || done == tiles.len() {
                writer.report_progress(
                    MapStage::Publishing,
                    done as u32,
                    bundle.tiles,
                    format!("installing shipped terrain · {done}/{}", bundle.tiles),
                );
            }
        }
        Ok(Installed::Published(
            writer.finish(BakeOutput::Images(manifest))?,
        ))
    }
}

enum TileFailure {
    /// The shipped tile is missing or damaged, or disagrees with the
    /// checkpoint: the shipped terrain is unusable.
    Shipped(MapCacheError),
    /// Writing into the cache failed, or the demand was cancelled.
    Cache(MapCacheError),
}

fn install_tile(
    source: &Path,
    tile: &TileReceipt,
    writer: &mut BakeWriter,
    buffer: &mut Vec<u8>,
) -> Result<(), TileFailure> {
    let unit = UnitKey::Terrain { tile: tile.key };
    if let Some(recorded) = writer.completed_receipt(unit) {
        // Checkpointed earlier; the writer re-verified its bytes on open.
        return if recorded == tile.payload {
            Ok(())
        } else {
            Err(TileFailure::Shipped(
                MapError::Invalid("checkpointed tile differs from the shipped tile").into(),
            ))
        };
    }
    let relative = tile
        .key
        .relative_path()
        .map_err(|error| TileFailure::Shipped(error.into()))?;
    read_tile(&source.join(&relative), tile, buffer).map_err(TileFailure::Shipped)?;
    atomic_write(&writer.directory().join(&relative), buffer).map_err(TileFailure::Cache)?;
    writer
        .record_unit(unit, tile.payload)
        .map_err(TileFailure::Cache)?;
    Ok(())
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
