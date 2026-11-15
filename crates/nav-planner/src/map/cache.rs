//! Typed publication boundary. Only checked complete directories produce ready
//! readers. A .<key>.partial directory can load checkpoints, never ready imagery.
//! D owns locks/atomic rename, demand cancellation and the PreparedRuntimeCache Arc.
use super::formats::{
    self, CatalogueManifest, ClientPois, ImageManifest, PayloadReceipt, TileReceipt,
    MAX_IMAGE_BYTES, MAX_IMAGE_TILES, MAX_JSON_BYTES, MAX_PNG_BYTES,
};
use super::identity::{CatalogueIdentity, Digest, ImageIdentity};
use super::spatial::TileKey;
use super::{MapError, Rows};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const CHECKPOINT_SCHEMA: u16 = 1;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "artifact",
    content = "identity",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ArtifactIdentity {
    Image(ImageIdentity),
    Catalogue(CatalogueIdentity),
}
impl ArtifactIdentity {
    pub fn revision(self) -> u16 {
        match self {
            Self::Image(identity) => identity.revision,
            Self::Catalogue(identity) => identity.revision,
        }
    }

    pub fn key(self) -> Result<Digest, MapError> {
        match self {
            Self::Image(i) => Ok(i.key()?.0),
            Self::Catalogue(i) => Ok(i.key()?.0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "unit", rename_all = "snake_case", deny_unknown_fields)]
pub enum UnitKey {
    ClientPois,
    Terrain { tile: TileKey },
}
impl UnitKey {
    fn relative_path(self) -> Result<PathBuf, MapError> {
        match self {
            Self::ClientPois => Ok("client-pois.json".into()),
            Self::Terrain { tile } => tile.relative_path(),
        }
    }
    fn byte_limit(self) -> u32 {
        match self {
            Self::ClientPois => MAX_JSON_BYTES as u32,
            Self::Terrain { .. } => MAX_PNG_BYTES,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompletedUnit {
    pub key: UnitKey,
    pub payload: PayloadReceipt,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BakeStage {
    Catalogue,
    BaseTerrain,
    Downsample,
    Publishing,
}

/// Checkpoint identity is retained as map metadata. Resume checks the cache
/// format, revision, unit shape and payload syntax, not content identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checkpoint {
    pub schema: u16,
    pub identity: ArtifactIdentity,
    pub stage: BakeStage,
    pub planned_units: u32,
    pub completed: Rows<CompletedUnit, MAX_IMAGE_TILES>,
}
impl Checkpoint {
    pub fn decode(bytes: &[u8]) -> Result<Self, MapError> {
        super::preflight(bytes, MAX_JSON_BYTES, "map checkpoint", CHECKPOINT_SCHEMA)?;
        let checkpoint: Self = super::json(bytes, MAX_JSON_BYTES)?;
        checkpoint.validate()?;
        Ok(checkpoint)
    }
    pub fn validate(&self) -> Result<(), MapError> {
        super::version("map checkpoint", self.schema, CHECKPOINT_SCHEMA)?;
        super::revision(self.identity.revision())?;
        if self.planned_units == 0
            || self.planned_units as usize > MAX_IMAGE_TILES
            || self.completed.as_slice().len() > self.planned_units as usize
        {
            return Err(MapError::Limit("checkpoint unit count"));
        }
        match self.identity {
            ArtifactIdentity::Catalogue(_)
                if self.planned_units != 1
                    || !matches!(self.stage, BakeStage::Catalogue | BakeStage::Publishing) =>
            {
                return Err(MapError::Invalid("catalogue checkpoint stage/count"))
            }
            ArtifactIdentity::Image(_) if self.stage == BakeStage::Catalogue => {
                return Err(MapError::Invalid("image checkpoint stage"))
            }
            _ => {}
        }
        let mut previous = None;
        for unit in self.completed.as_slice() {
            formats::ordered(previous, unit.key, "checkpoint unit")?;
            previous = Some(unit.key);
            match (self.identity, unit.key) {
                (ArtifactIdentity::Catalogue(_), UnitKey::ClientPois) => {}
                (ArtifactIdentity::Image(_), UnitKey::Terrain { tile }) => {
                    tile.bounds()?;
                }
                _ => return Err(MapError::Invalid("checkpoint artifact/unit")),
            }
        }
        Ok(())
    }
    pub fn encode(&self) -> Result<Vec<u8>, MapError> {
        self.validate()?;
        formats::encode_json(self)
    }
}

/// Only partial publication paths have this type. No conversion to ready: after
/// D atomically publishes, reopen using ReadyImages/ReadyCatalogue's checked load.
#[derive(Debug)]
pub struct PartialEntry {
    directory: PathBuf,
    identity: ArtifactIdentity,
}
impl PartialEntry {
    pub fn at(directory: &Path, identity: ArtifactIdentity) -> Result<Self, MapError> {
        check_name(directory, true)?;
        Ok(Self {
            directory: directory.to_owned(),
            identity,
        })
    }
    pub fn directory(&self) -> &Path {
        &self.directory
    }
    pub fn identity(&self) -> ArtifactIdentity {
        self.identity
    }
    /// Resume re-decodes every completed unit before exposing its checkpoint.
    /// Ready opens decode the manifest and only requested payloads.
    pub fn load_checkpoint(&self) -> Result<Checkpoint, MapError> {
        let mut checkpoint = Checkpoint::decode(&read_file(
            &self.directory.join("checkpoint.json"),
            MAX_JSON_BYTES,
        )?)?;
        if checkpoint.identity.revision() != self.identity.revision() {
            return Err(MapError::Invalid("checkpoint revision"));
        }
        let mut buffer = Vec::new();
        let mut total_bytes = 0u64;
        let mut completed = Vec::with_capacity(checkpoint.completed.as_slice().len());
        for unit in checkpoint.completed.as_slice() {
            read_into(
                &self.directory.join(unit.key.relative_path()?),
                unit.key.byte_limit() as usize,
                &mut buffer,
            )?;
            match (self.identity, unit.key) {
                (ArtifactIdentity::Catalogue(_), UnitKey::ClientPois) => {
                    ClientPois::decode(&buffer)?;
                }
                (_, UnitKey::Terrain { tile }) => {
                    TileReceipt::validate_png_bytes(tile, &buffer)?;
                }
                _ => return Err(MapError::Invalid("checkpoint unit")),
            }
            total_bytes = total_bytes
                .checked_add(buffer.len() as u64)
                .ok_or(MapError::Limit("checkpoint disk bytes"))?;
            if total_bytes > MAX_IMAGE_BYTES {
                return Err(MapError::Limit("checkpoint disk bytes"));
            }
            completed.push(CompletedUnit {
                key: unit.key,
                payload: PayloadReceipt {
                    bytes: u32::try_from(buffer.len())
                        .map_err(|_| MapError::Limit("checkpoint unit bytes"))?,
                    sha256: Digest::of(&buffer),
                },
            });
        }
        checkpoint.completed = Rows::new(completed)?;
        Ok(checkpoint)
    }
}

#[derive(Debug)]
pub struct ReadyImages {
    directory: PathBuf,
    manifest: ImageManifest,
}
impl ReadyImages {
    pub fn open(directory: &Path) -> Result<Self, MapError> {
        check_name(directory, false)?;
        let manifest = ImageManifest::decode(&read_file(
            &directory.join("manifest.json"),
            MAX_JSON_BYTES,
        )?)?;
        Ok(Self {
            directory: directory.to_owned(),
            manifest,
        })
    }
    pub fn manifest(&self) -> &ImageManifest {
        &self.manifest
    }
    /// None is unlisted/no art. A listed missing/corrupt payload is an error.
    /// Caller reuses this compressed buffer and caps the PNG decoder to the
    /// returned decoded byte count before allocating RGBA memory.
    pub fn read_tile_into(
        &self,
        key: TileKey,
        buffer: &mut Vec<u8>,
    ) -> Result<Option<usize>, MapError> {
        if self.manifest.tile(key).is_none() {
            return Ok(None);
        }
        read_into(
            &self.directory.join(key.relative_path()?),
            MAX_PNG_BYTES as usize,
            buffer,
        )?;
        Ok(Some(TileReceipt::validate_png_bytes(key, buffer)?))
    }
    /// None is unlisted/no art. A listed missing or corrupt payload is an error.
    /// The format limit bounds allocation before the PNG is read.
    pub fn read_tile_arc(&self, key: TileKey) -> Result<Option<Arc<[u8]>>, MapError> {
        if self.manifest.tile(key).is_none() {
            return Ok(None);
        }
        let path = self.directory.join(key.relative_path()?);
        let mut file = File::open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(MapError::Path);
        }
        if metadata.len() > u64::from(MAX_PNG_BYTES) {
            return Err(MapError::Limit("file bytes"));
        }
        let length = usize::try_from(metadata.len()).map_err(|_| MapError::Limit("file bytes"))?;
        let mut bytes: Arc<[u8]> = std::iter::repeat_n(0u8, length).collect();
        let output = Arc::get_mut(&mut bytes).expect("new Arc is uniquely owned");
        file.read_exact(output).map_err(|error| {
            if error.kind() == std::io::ErrorKind::UnexpectedEof {
                MapError::Truncated
            } else {
                error.into()
            }
        })?;
        let mut trailing = [0u8; 1];
        if file.read(&mut trailing)? != 0 {
            return Err(MapError::Invalid("file changed while reading"));
        }
        TileReceipt::validate_png_bytes(key, &bytes)?;
        Ok(Some(bytes))
    }
}

#[derive(Debug)]
pub struct ReadyCatalogue {
    manifest: CatalogueManifest,
    pois: ClientPois,
}
impl ReadyCatalogue {
    pub fn open(directory: &Path) -> Result<Self, MapError> {
        check_name(directory, false)?;
        let manifest = CatalogueManifest::decode(&read_file(
            &directory.join("manifest.json"),
            MAX_JSON_BYTES,
        )?)?;
        let pois = manifest.read_payload(&read_file(
            &directory.join("client-pois.json"),
            MAX_JSON_BYTES,
        )?)?;
        Ok(Self { manifest, pois })
    }
    pub fn manifest(&self) -> &CatalogueManifest {
        &self.manifest
    }
    pub fn pois(&self) -> &ClientPois {
        &self.pois
    }
}

fn check_name(directory: &Path, partial: bool) -> Result<(), MapError> {
    let name = directory
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or(MapError::Path)?;
    let partial_key = name
        .strip_prefix('.')
        .and_then(|name| name.strip_suffix(".partial"));
    if partial {
        if partial_key.is_none_or(str::is_empty) {
            return Err(MapError::Path);
        }
    } else if name.ends_with(".partial") {
        return Err(MapError::NotReady);
    }
    Ok(())
}
fn read_file(path: &Path, limit: usize) -> Result<Vec<u8>, MapError> {
    let mut buffer = Vec::new();
    read_into(path, limit, &mut buffer)?;
    Ok(buffer)
}
fn read_into(path: &Path, limit: usize, buffer: &mut Vec<u8>) -> Result<(), MapError> {
    let mut file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(MapError::Path);
    }
    if metadata.len() > limit as u64 {
        return Err(MapError::Limit("file bytes"));
    }
    buffer.resize(metadata.len() as usize, 0);
    file.read_exact(buffer).map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            MapError::Truncated
        } else {
            e.into()
        }
    })?;
    if file.read(&mut [0])? != 0 {
        return Err(MapError::Invalid("file changed while reading"));
    }
    Ok(())
}
