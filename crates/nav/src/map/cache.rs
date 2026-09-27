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

/// Identity includes the policy (and therefore exact encoder versions). A
/// completed unit is resumable only after its length/digest and schema recheck.
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
    pub fn decode(bytes: &[u8], expected: ArtifactIdentity) -> Result<Self, MapError> {
        super::preflight(
            bytes,
            MAX_JSON_BYTES,
            expected,
            "map checkpoint",
            CHECKPOINT_SCHEMA,
        )?;
        let checkpoint: Self = super::json(bytes, MAX_JSON_BYTES)?;
        checkpoint.validate(expected)?;
        Ok(checkpoint)
    }
    pub fn validate(&self, expected: ArtifactIdentity) -> Result<(), MapError> {
        super::version("map checkpoint", self.schema, CHECKPOINT_SCHEMA)?;
        self.identity.key()?;
        if self.identity != expected {
            return Err(MapError::Identity);
        }
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
        let mut bytes = 0u64;
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
            unit.payload.validate(unit.key.byte_limit())?;
            bytes += u64::from(unit.payload.bytes);
            if bytes > MAX_IMAGE_BYTES {
                return Err(MapError::Limit("checkpoint disk bytes"));
            }
        }
        Ok(())
    }
    pub fn encode(&self) -> Result<Vec<u8>, MapError> {
        self.validate(self.identity)?;
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
        check_name(directory, identity.key()?, true)?;
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
    /// Resume is the only path that verifies all completed units. Ordinary ready
    /// open verifies a manifest, then only requested payloads, never the pyramid.
    pub fn load_checkpoint(&self) -> Result<Checkpoint, MapError> {
        let checkpoint = Checkpoint::decode(
            &read_file(&self.directory.join("checkpoint.json"), MAX_JSON_BYTES)?,
            self.identity,
        )?;
        let mut buffer = Vec::new();
        for unit in checkpoint.completed.as_slice() {
            read_into(
                &self.directory.join(unit.key.relative_path()?),
                unit.key.byte_limit() as usize,
                &mut buffer,
            )?;
            match (self.identity, unit.key) {
                (ArtifactIdentity::Catalogue(identity), UnitKey::ClientPois) => {
                    unit.payload.verify(&buffer, MAX_JSON_BYTES as u32)?;
                    ClientPois::decode(&buffer, identity)?;
                }
                (_, UnitKey::Terrain { tile }) => {
                    TileReceipt {
                        key: tile,
                        payload: unit.payload,
                    }
                    .verify_png(&buffer)?;
                }
                _ => return Err(MapError::Invalid("checkpoint unit")),
            }
        }
        Ok(checkpoint)
    }
}

#[derive(Debug)]
pub struct ReadyImages {
    directory: PathBuf,
    manifest: ImageManifest,
}
impl ReadyImages {
    pub fn open(directory: &Path, expected: ImageIdentity) -> Result<Self, MapError> {
        check_name(directory, expected.key()?.0, false)?;
        let manifest = ImageManifest::decode(
            &read_file(&directory.join("manifest.json"), MAX_JSON_BYTES)?,
            expected,
        )?;
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
        let Some(receipt) = self.manifest.tile(key) else {
            return Ok(None);
        };
        read_into(
            &self.directory.join(key.relative_path()?),
            receipt.payload.bytes as usize,
            buffer,
        )?;
        Ok(Some(receipt.verify_png(buffer)?))
    }
}

#[derive(Debug)]
pub struct ReadyCatalogue {
    manifest: CatalogueManifest,
    pois: ClientPois,
}
impl ReadyCatalogue {
    pub fn open(directory: &Path, expected: CatalogueIdentity) -> Result<Self, MapError> {
        check_name(directory, expected.key()?.0, false)?;
        let manifest = CatalogueManifest::decode(
            &read_file(&directory.join("manifest.json"), MAX_JSON_BYTES)?,
            expected,
        )?;
        let pois = manifest.read_payload(&read_file(
            &directory.join("client-pois.json"),
            manifest.payload.bytes as usize,
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

fn check_name(directory: &Path, key: Digest, partial: bool) -> Result<(), MapError> {
    let name = directory
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or(MapError::Path)?;
    if !partial && name.ends_with(".partial") {
        return Err(MapError::NotReady);
    }
    let expected = if partial {
        format!(".{key}.partial")
    } else {
        key.to_string()
    };
    if name != expected {
        return Err(MapError::Path);
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
