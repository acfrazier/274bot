//! Persistent, demand-driven WalkTo map resources.
//!
//! This module is deliberately host-owned: profile preparation only creates an
//! immutable [`MapProfileDescriptor`].  The first explicit demand acquires a
//! worker and a second `Arc` lease on the prepared client cache.  Workers write
//! only identity-keyed `.partial` directories and publish a complete artifact
//! with one cross-process lock; readers never inspect a partial directory.
//!
//! The producer is injected so the lifecycle can be qualified with a small
//! real raster fixture and later connected to the native client decoder/raster
//! without moving ownership into a bot slot or profile preparation.

use client::unpack::PreparedRuntimeCache;
use nav::map::cache::{
    ArtifactIdentity, BakeStage, Checkpoint, CompletedUnit, PartialEntry,
    ReadyCatalogue as NavReadyCatalogue, ReadyImages as NavReadyImages, UnitKey, CHECKPOINT_SCHEMA,
};
use nav::map::formats::{
    CatalogueManifest, ClientPois, ImageManifest, PayloadReceipt, TileReceipt, MAX_IMAGE_BYTES,
    MAX_IMAGE_TILES, MAX_JSON_BYTES,
};
use nav::map::identity::{CatalogueIdentity, Digest, ImageIdentity};
use nav::map::MapError;
use parking_lot::Mutex;
use serde::Serialize;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering as AtomicOrdering};
use std::sync::{Arc, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime};

pub const PUBLISH_LOCK_NAME: &str = ".publish.lock";
pub const MAX_GENERATED_CACHE_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_IMAGE_KEY_BYTES: u64 = 128 * 1024 * 1024;
const CHECKPOINT_INTERVAL: usize = 32;
const LOCK_RETRY: Duration = Duration::from_millis(10);
const MAX_CATALOGUE_KEY_BYTES: u64 = MAX_JSON_BYTES as u64;
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Error returned by cache layout, publication and worker ownership code.
#[derive(Debug)]
pub enum MapCacheError {
    Io(io::Error),
    Map(MapError),
    Message(String),
    Cancelled,
    NoPreparedCache,
    Busy,
}

impl fmt::Display for MapCacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "map cache I/O: {error}"),
            Self::Map(error) => write!(f, "map cache format: {error}"),
            Self::Message(message) => f.write_str(message),
            Self::Cancelled => f.write_str("map bake cancelled"),
            Self::NoPreparedCache => {
                f.write_str("map assets unavailable: prepared client cache is absent")
            }
            Self::Busy => f.write_str("map cache worker is busy with another identity"),
        }
    }
}

impl std::error::Error for MapCacheError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Map(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for MapCacheError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}
impl From<MapError> for MapCacheError {
    fn from(error: MapError) -> Self {
        Self::Map(error)
    }
}

#[cfg(test)]
#[derive(Debug, PartialEq, Eq)]
struct TestCleanup(Option<PathBuf>);

#[cfg(test)]
impl Drop for TestCleanup {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            let _ = fs::remove_dir_all(path);
        }
    }
}

/// Resolved `~/.274bot/map-cache` root.  Tests use [`Self::from_home`] so no
/// process environment mutation is needed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapCacheRoot(PathBuf, #[cfg(test)] Arc<TestCleanup>);

impl MapCacheRoot {
    pub fn from_home(home: impl AsRef<Path>) -> Self {
        Self::from_root(home.as_ref().join(".274bot").join("map-cache"))
    }

    pub fn from_root(root: impl Into<PathBuf>) -> Self {
        let path = root.into();
        Self(
            path.clone(),
            #[cfg(test)]
            Arc::new(TestCleanup(Some(path))),
        )
    }

    #[cfg(test)]
    // Child processes in the shared-root stress test must not remove the
    // parent-owned cache when their local root handle is dropped.
    fn from_existing_root(root: impl Into<PathBuf>) -> Self {
        Self(root.into(), Arc::new(TestCleanup(None)))
    }

    pub fn current() -> Result<Self, MapCacheError> {
        let home = client::operator_home().map_err(|error| {
            MapCacheError::Message(format!(
                "cannot resolve operator home for map cache: {error}"
            ))
        })?;
        Ok(Self::from_home(home))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn revision_dir(&self, revision: u16) -> PathBuf {
        self.0.join(revision.to_string())
    }

    fn kind_dir(&self, revision: u16, kind: ArtifactKind) -> PathBuf {
        self.revision_dir(revision).join(kind.directory_name())
    }

    pub fn catalogue_dir(&self, identity: CatalogueIdentity) -> Result<PathBuf, MapCacheError> {
        Ok(self
            .kind_dir(identity.revision, ArtifactKind::Catalogue)
            .join(identity.key()?.0.to_string()))
    }

    pub fn image_dir(&self, identity: ImageIdentity) -> Result<PathBuf, MapCacheError> {
        Ok(self
            .kind_dir(identity.revision, ArtifactKind::Images)
            .join(identity.key()?.0.to_string()))
    }

    pub fn partial_dir(&self, revision: u16, kind: ArtifactKind, key: Digest) -> PathBuf {
        self.kind_dir(revision, kind)
            .join(format!(".{key}.partial"))
    }

    fn lock_path(&self, revision: u16, kind: ArtifactKind, key: Digest) -> PathBuf {
        self.kind_dir(revision, kind)
            .join(format!(".{key}{PUBLISH_LOCK_NAME}"))
    }

    fn ensure_kind_dir(&self, revision: u16, kind: ArtifactKind) -> Result<PathBuf, MapCacheError> {
        let path = self.kind_dir(revision, kind);
        if path.exists() && !real_directory(&path)? {
            return Err(MapCacheError::Map(MapError::Path));
        }
        fs::create_dir_all(&path)?;
        Ok(path)
    }

    /// Direct ready lookup.  This reads bounded metadata only and never walks
    /// the generated-cache tree.
    pub fn open_catalogue(
        &self,
        identity: CatalogueIdentity,
    ) -> Result<Option<Arc<ReadyCatalogue>>, MapCacheError> {
        let path = self.catalogue_dir(identity)?;
        let lock_path = self.lock_path(
            identity.revision,
            ArtifactKind::Catalogue,
            identity.key()?.0,
        );
        let Some(lock) = ready_lease(&lock_path, &path)? else {
            return Ok(None);
        };
        clear_checkpoint(&path)?;
        match ReadyCatalogue::open(&path, identity, lock) {
            Ok(value) => Ok(Some(Arc::new(value))),
            Err(MapCacheError::Map(MapError::Io(error)))
                if error.kind() == io::ErrorKind::NotFound =>
            {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    /// Direct ready lookup.  Tile PNGs remain unopened until a renderer asks
    /// `ReadyImages::read_tile_into` for one visible key.
    pub fn open_images(
        &self,
        identity: ImageIdentity,
    ) -> Result<Option<Arc<ReadyImages>>, MapCacheError> {
        let path = self.image_dir(identity)?;
        let lock_path = self.lock_path(identity.revision, ArtifactKind::Images, identity.key()?.0);
        let Some(lock) = ready_lease(&lock_path, &path)? else {
            return Ok(None);
        };
        clear_checkpoint(&path)?;
        match ReadyImages::open(&path, identity, lock) {
            Ok(value) => Ok(Some(Arc::new(value))),
            Err(MapCacheError::Map(MapError::Io(error)))
                if error.kind() == io::ErrorKind::NotFound =>
            {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    /// Count generated bytes for an explicit capacity check.  This traverses
    /// only our generated root and never the client snapshot/nav directories.
    pub fn generated_bytes(&self) -> Result<u64, MapCacheError> {
        if !self.0.exists() {
            return Ok(0);
        }
        Ok(directory_bytes(&self.0)?)
    }

    /// Remove least-recently-used inactive generated entries until the global
    /// cap is met.  Required keys and any directory with an active lock are
    /// never removed.  No path outside `map-cache` can be reached here.
    pub fn prune_inactive(&self, required: &BTreeSet<PathBuf>) -> Result<u64, MapCacheError> {
        let capacity_lock = self.0.join(format!(".capacity{PUBLISH_LOCK_NAME}"));
        let capacity = PublishLock::acquire(&capacity_lock, None)?;
        let result = self.prune_to(required, MAX_GENERATED_CACHE_BYTES);
        drop(capacity);
        result
    }

    fn prune_to(&self, required: &BTreeSet<PathBuf>, limit: u64) -> Result<u64, MapCacheError> {
        let mut usage = self.generated_bytes()?;
        if usage <= limit {
            return Ok(0);
        }
        let mut candidates = Vec::new();
        collect_ready_entries(&self.0, required, &mut candidates)?;
        candidates.sort_by(|a, b| match a.0.cmp(&b.0) {
            Ordering::Equal => a.1.cmp(&b.1),
            order => order,
        });
        let mut removed = 0;
        for (_, path, bytes) in candidates {
            if usage <= limit {
                break;
            }
            let lock_path = entry_lock_path(&path).ok_or(MapCacheError::Map(MapError::Path))?;
            let Some(lock) = (match PublishLock::try_acquire(&lock_path) {
                Ok(lock) => lock,
                Err(_) => continue,
            }) else {
                continue;
            };
            if !path.exists() {
                drop(lock);
                continue;
            }
            fs::remove_dir_all(&path)?;
            // The lock file itself is never unlinked: a process that opened
            // it before an unlink would lock an orphaned inode while another
            // creates and locks a fresh one, giving the key two owners.
            drop(lock);
            usage = usage.saturating_sub(bytes);
            removed += bytes;
        }
        Ok(removed)
    }

    fn ensure_capacity(
        &self,
        additional: u64,
        required: &BTreeSet<PathBuf>,
        cancel: Option<&AtomicBool>,
    ) -> Result<(), MapCacheError> {
        let capacity_lock = self.0.join(format!(".capacity{PUBLISH_LOCK_NAME}"));
        let capacity = PublishLock::acquire(&capacity_lock, cancel)?;
        let result = self.ensure_capacity_locked(additional, required);
        drop(capacity);
        result
    }

    fn ensure_capacity_locked(
        &self,
        additional: u64,
        required: &BTreeSet<PathBuf>,
    ) -> Result<(), MapCacheError> {
        let usage = self.generated_bytes()?;
        if usage.saturating_add(additional) <= MAX_GENERATED_CACHE_BYTES {
            return Ok(());
        }
        let limit = MAX_GENERATED_CACHE_BYTES.saturating_sub(additional);
        self.prune_to(required, limit)?;
        if self.generated_bytes()?.saturating_add(additional) > MAX_GENERATED_CACHE_BYTES {
            return Err(MapCacheError::Map(MapError::Limit(
                "generated map cache bytes",
            )));
        }
        Ok(())
    }
}

/// Ready catalogue data plus the shared lock that protects its directory from
/// cross-process pruning while any consumer retains the Arc.
#[derive(Debug)]
pub struct ReadyCatalogue {
    inner: NavReadyCatalogue,
    _lock: PublishLock,
}

impl ReadyCatalogue {
    fn open(
        directory: &Path,
        identity: CatalogueIdentity,
        lock: PublishLock,
    ) -> Result<Self, MapCacheError> {
        Ok(Self {
            inner: NavReadyCatalogue::open(directory, identity)?,
            _lock: lock,
        })
    }

    pub fn manifest(&self) -> &CatalogueManifest {
        self.inner.manifest()
    }

    pub fn pois(&self) -> &ClientPois {
        self.inner.pois()
    }
}

/// Ready terrain metadata plus the shared lock that protects its directory
/// from cross-process pruning while any consumer retains the Arc.
#[derive(Debug)]
pub struct ReadyImages {
    inner: NavReadyImages,
    _lock: PublishLock,
}

impl ReadyImages {
    fn open(
        directory: &Path,
        identity: ImageIdentity,
        lock: PublishLock,
    ) -> Result<Self, MapCacheError> {
        Ok(Self {
            inner: NavReadyImages::open(directory, identity)?,
            _lock: lock,
        })
    }

    pub fn manifest(&self) -> &ImageManifest {
        self.inner.manifest()
    }

    pub fn read_tile_into(
        &self,
        key: nav::map::spatial::TileKey,
        buffer: &mut Vec<u8>,
    ) -> Result<Option<usize>, MapCacheError> {
        Ok(self.inner.read_tile_into(key, buffer)?)
    }
}

fn directory_bytes(path: &Path) -> io::Result<u64> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() {
        return Ok(0);
    }

    if metadata.is_file() {
        return Ok(metadata.len());
    }
    if !metadata.is_dir() {
        return Ok(0);
    }
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    let mut bytes: u64 = 0;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        bytes = bytes.saturating_add(directory_bytes(&entry.path())?);
    }
    Ok(bytes)
}

fn entry_lock_path(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_str()?;
    let key = name
        .strip_prefix('.')
        .and_then(|name| name.strip_suffix(".partial"))
        .unwrap_or(name);
    if key.is_empty() {
        return None;
    }
    Some(path.parent()?.join(format!(".{key}{PUBLISH_LOCK_NAME}")))
}

fn collect_ready_entries(
    root: &Path,
    required: &BTreeSet<PathBuf>,
    out: &mut Vec<(SystemTime, PathBuf, u64)>,
) -> io::Result<()> {
    let root_metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Ok(());
    }
    let revisions = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for revision in revisions {
        let revision = match revision {
            Ok(entry) => entry.path(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        let metadata = match fs::symlink_metadata(&revision) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            continue;
        }
        let kinds = match fs::read_dir(&revision) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        for kind in kinds {
            let kind = match kind {
                Ok(entry) => entry.path(),
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            let metadata = match fs::symlink_metadata(&kind) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                continue;
            }
            let entries = match fs::read_dir(&kind) {
                Ok(entries) => entries,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            for entry in entries {
                let path = match entry {
                    Ok(entry) => entry.path(),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(error),
                };
                let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                let metadata = match fs::symlink_metadata(&path) {
                    Ok(metadata) => metadata,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(error),
                };
                let is_partial = name.starts_with('.') && name.ends_with(".partial");
                if (!is_partial && name.starts_with('.'))
                    || required.contains(&path)
                    || metadata.file_type().is_symlink()
                    || !metadata.is_dir()
                {
                    continue;
                }
                let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                out.push((modified, path.clone(), directory_bytes(&path)?));
            }
        }
    }
    Ok(())
}
/// Shared lease on a published directory, or `None` while the key is held
/// exclusively (publishing or pruning) or the directory is absent.  The lock is
/// taken first so a pruner cannot remove the directory after the check.
fn ready_lease(lock_path: &Path, ready: &Path) -> Result<Option<PublishLock>, MapCacheError> {
    let Some(lease) = PublishLock::try_acquire_shared(lock_path)? else {
        return Ok(None);
    };
    Ok(real_directory(ready)?.then_some(lease))
}

fn real_directory(path: &Path) -> Result<bool, MapCacheError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(MapCacheError::Map(MapError::Path))
        }
        Ok(metadata) => {
            if metadata.is_dir() {
                Ok(true)
            } else {
                Err(MapCacheError::Map(MapError::Path))
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactKind {
    Catalogue,
    Images,
}
impl ArtifactKind {
    fn directory_name(self) -> &'static str {
        match self {
            Self::Catalogue => "catalogues",
            Self::Images => "images",
        }
    }
}

/// Immutable map input binding.  Cloning this lease clones the owning
/// `Arc<PreparedRuntimeCache>`; no worker stores only `jag_dir` or
/// `snapshot_dir` after a profile can be rebound.
#[derive(Clone)]
pub struct PreparedMapInput {
    revision: u16,
    content: Digest,
    prepared: Arc<PreparedRuntimeCache>,
}

impl fmt::Debug for PreparedMapInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedMapInput")
            .field("revision", &self.revision)
            .field("content", &self.content)
            .field("prepared", &"Arc<PreparedRuntimeCache>")
            .finish()
    }
}

impl PreparedMapInput {
    pub fn from_prepared(
        revision: u16,
        prepared: Arc<PreparedRuntimeCache>,
    ) -> Result<Self, MapCacheError> {
        if prepared.identity.revision != revision {
            return Err(MapCacheError::Message(format!(
                "prepared map revision {} does not match profile revision {revision}",
                prepared.identity.revision
            )));
        }
        Ok(Self {
            revision,
            content: Digest(prepared.identity.content_id),
            prepared,
        })
    }

    pub fn revision(&self) -> u16 {
        self.revision
    }
    pub fn content(&self) -> Digest {
        self.content
    }
    pub fn prepared_cache(&self) -> &Arc<PreparedRuntimeCache> {
        &self.prepared
    }
    pub fn clone_prepared_cache(&self) -> Arc<PreparedRuntimeCache> {
        Arc::clone(&self.prepared)
    }
    pub fn jag_dir(&self) -> &Path {
        &self.prepared.jag_dir
    }
    pub fn snapshot_dir(&self) -> &Path {
        &self.prepared.snapshot_dir
    }
}

/// Lightweight profile binding.  Constructing this value performs no cache
/// scan, POI derivation, PNG work, texture registration, or worker spawn.
#[derive(Clone)]
pub struct MapProfileDescriptor {
    revision: u16,
    content: Digest,
    image: ImageIdentity,
    catalogue: CatalogueIdentity,
    input: Option<PreparedMapInput>,
}

impl fmt::Debug for MapProfileDescriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MapProfileDescriptor")
            .field("revision", &self.revision)
            .field("content", &self.content)
            .field("image", &self.image)
            .field("catalogue", &self.catalogue)
            .field("has_input_lease", &self.input.is_some())
            .finish()
    }
}

impl MapProfileDescriptor {
    pub fn new(
        revision: u16,
        content: Digest,
        image_policy: Digest,
        catalogue_policy: Digest,
    ) -> Result<Self, MapCacheError> {
        let image = ImageIdentity {
            revision,
            content,
            policy: image_policy,
        };
        let catalogue = CatalogueIdentity {
            revision,
            content,
            policy: catalogue_policy,
        };
        image.key()?;
        catalogue.key()?;
        Ok(Self {
            revision,
            content,
            image,
            catalogue,
            input: None,
        })
    }

    pub fn with_input(mut self, input: PreparedMapInput) -> Result<Self, MapCacheError> {
        if input.revision != self.revision || input.content != self.content {
            return Err(MapCacheError::Message(
                "prepared map input does not match descriptor identity".into(),
            ));
        }
        self.input = Some(input);
        Ok(self)
    }
    fn without_input(mut self) -> Self {
        self.input = None;
        self
    }

    pub fn from_profile(
        profile: &crate::ServerProfile,
        image_policy: Digest,
        catalogue_policy: Digest,
    ) -> Result<Self, MapCacheError> {
        let revision = profile.revision().as_i32() as u16;
        let prepared = profile
            .prepared_cache()
            .ok_or(MapCacheError::NoPreparedCache)?;
        let input = PreparedMapInput::from_prepared(revision, Arc::clone(prepared))?;
        Self::new(revision, input.content(), image_policy, catalogue_policy)?.with_input(input)
    }

    pub fn revision(&self) -> u16 {
        self.revision
    }
    pub fn content(&self) -> Digest {
        self.content
    }
    pub fn image_identity(&self) -> ImageIdentity {
        self.image
    }
    pub fn catalogue_identity(&self) -> CatalogueIdentity {
        self.catalogue
    }
    pub fn input(&self) -> Option<&PreparedMapInput> {
        self.input.as_ref()
    }
    pub fn image_key(&self) -> Result<Digest, MapCacheError> {
        Ok(self.image.key()?.0)
    }
    pub fn catalogue_key(&self) -> Result<Digest, MapCacheError> {
        Ok(self.catalogue.key()?.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapDemand {
    CatalogueOnly,
    Images,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapStage {
    ReadingCache,
    DerivingPois,
    BakingPlane,
    BuildingZoomLevels,
    Publishing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapProgress {
    pub stage: MapStage,
    pub completed: u32,
    pub total: u32,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapJobStatus {
    Queued,
    Running(MapProgress),
    Ready,
    Paused,
    Cancelled,
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BakePlan {
    pub planned_units: u32,
    pub stage: BakeStage,
}

impl BakePlan {
    pub fn checked(self) -> Result<Self, MapCacheError> {
        if self.planned_units == 0 || self.planned_units as usize > MAX_IMAGE_TILES {
            return Err(MapCacheError::Map(MapError::Limit("bake unit count")));
        }
        Ok(self)
    }
}

/// A worker request passed to B's decoder/raster producer.  `input` owns the
/// profile's second Arc lease for the complete worker lifetime.
#[derive(Clone)]
pub struct BakeRequest {
    artifact: ArtifactKind,
    descriptor: MapProfileDescriptor,
}

impl fmt::Debug for BakeRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BakeRequest")
            .field("artifact", &self.artifact)
            .field("descriptor", &self.descriptor)
            .finish()
    }
}

impl BakeRequest {
    pub fn artifact(&self) -> ArtifactKind {
        self.artifact
    }
    pub fn descriptor(&self) -> &MapProfileDescriptor {
        &self.descriptor
    }
    pub fn input(&self) -> Option<&PreparedMapInput> {
        self.descriptor.input()
    }
    /// Borrow the verified client snapshot for one producer call. The
    /// descriptor's prepared-cache Arc remains owned by the worker request,
    /// so the returned paths cannot outlive that lease.
    pub fn client_input<'a>(
        &'a self,
    ) -> Result<nav::map::producer::ClientMapInput<'a>, MapCacheError> {
        let input = self
            .descriptor
            .input()
            .ok_or(MapCacheError::NoPreparedCache)?;
        nav::map::producer::ClientMapInput::new(
            input.revision(),
            input.content(),
            input.jag_dir(),
            input.snapshot_dir(),
        )
        .map_err(MapCacheError::Map)
    }
    pub fn expected_identity(&self) -> ArtifactIdentity {
        match self.artifact {
            ArtifactKind::Catalogue => {
                ArtifactIdentity::Catalogue(self.descriptor.catalogue_identity())
            }
            ArtifactKind::Images => ArtifactIdentity::Image(self.descriptor.image_identity()),
        }
    }
}

/// B owns format decoding/raster policy; D owns demand, persistence and
/// publication.  A producer must use [`BakeWriter::publish_unit`] for every
/// output payload and return a matching manifest from `run`.
pub trait MapBakeProducer: Send + Sync + 'static {
    fn plan(&self, request: &BakeRequest) -> Result<BakePlan, MapCacheError>;
    fn run(
        &self,
        request: &BakeRequest,
        writer: &mut BakeWriter,
    ) -> Result<BakeOutput, MapCacheError>;
}

pub enum BakeOutput {
    Catalogue(CatalogueManifest),
    Images(ImageManifest),
}

impl fmt::Debug for BakeOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Catalogue(manifest) => f.debug_tuple("Catalogue").field(manifest).finish(),
            Self::Images(manifest) => f.debug_tuple("Images").field(manifest).finish(),
        }
    }
}

/// Stateful persistence helper used by a producer.  Existing units are
/// verified through A's `PartialEntry` before this object is constructed, so
/// `publish_unit` can safely skip their raster work on resume.
pub struct BakeWriter {
    root: MapCacheRoot,
    partial: PathBuf,
    identity: ArtifactIdentity,
    checkpoint: Checkpoint,
    cancel: Arc<AtomicBool>,
    progress: Arc<dyn Fn(MapProgress) + Send + Sync>,
    required: BTreeSet<PathBuf>,
    bytes: u64,
    cache_bytes: u64,
    checkpoint_bytes: u64,
    checkpoint_dirty: bool,
}

impl fmt::Debug for BakeWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BakeWriter")
            .field("partial", &self.partial)
            .field("identity", &self.identity)
            .field("completed", &self.checkpoint.completed.as_slice().len())
            .field("planned", &self.checkpoint.planned_units)
            .finish()
    }
}
impl BakeWriter {
    fn open(
        root: MapCacheRoot,
        request: &BakeRequest,
        plan: BakePlan,
        cancel: Arc<AtomicBool>,
        progress: Arc<dyn Fn(MapProgress) + Send + Sync>,
        required: BTreeSet<PathBuf>,
    ) -> Result<Self, MapCacheError> {
        let identity = request.expected_identity();
        let key = identity.key()?;
        let partial = root.partial_dir(request.descriptor.revision(), request.artifact, key);
        root.ensure_kind_dir(request.descriptor.revision(), request.artifact)?;
        let mut checkpoint = None;
        if partial.exists() {
            if !real_directory(&partial)? {
                return Err(MapCacheError::Map(MapError::Path));
            }
            let entry = PartialEntry::at(&partial, identity);
            checkpoint = entry.ok().and_then(|entry| {
                let checkpoint = entry.load_checkpoint().ok()?;
                (checkpoint.planned_units == plan.planned_units).then_some(checkpoint)
            });
            if checkpoint.is_none() {
                fs::remove_dir_all(&partial)?;
            }
        }
        let mut required = required;
        required.insert(partial.clone());
        fs::create_dir_all(&partial)?;
        root.ensure_capacity(0, &required, Some(&cancel))?;
        let checkpoint = checkpoint.unwrap_or(Checkpoint {
            schema: CHECKPOINT_SCHEMA,
            identity,
            stage: plan.stage,
            planned_units: plan.planned_units,
            completed: nav::map::Rows::new(Vec::new())?,
        });
        let bytes = checkpoint
            .completed
            .as_slice()
            .iter()
            .map(|unit| u64::from(unit.payload.bytes))
            .sum();
        if bytes > max_artifact_bytes(request.artifact) {
            return Err(MapCacheError::Map(MapError::Limit("artifact bytes")));
        }
        let cache_bytes = root.generated_bytes()?;
        let checkpoint_bytes = fs::metadata(partial.join("checkpoint.json"))
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        let mut writer = Self {
            root,
            partial,
            identity,
            checkpoint,
            cancel,
            progress,
            required,
            bytes,
            cache_bytes,
            checkpoint_bytes,
            checkpoint_dirty: false,
        };
        writer.persist_checkpoint()?;
        Ok(writer)
    }

    pub fn artifact(&self) -> ArtifactKind {
        match self.identity {
            ArtifactIdentity::Catalogue(_) => ArtifactKind::Catalogue,
            ArtifactIdentity::Image(_) => ArtifactKind::Images,
        }
    }

    /// Partial directory this writer publishes into. Producers may write
    /// intermediate child tiles here for downsample; `publish_unit` remains
    /// the publication path.
    pub fn directory(&self) -> &Path {
        &self.partial
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(AtomicOrdering::Acquire)
    }

    pub fn checkpoint(&self) -> &Checkpoint {
        &self.checkpoint
    }

    pub fn should_skip(&self, key: UnitKey) -> bool {
        self.checkpoint
            .completed
            .as_slice()
            .iter()
            .any(|unit| unit.key == key)
    }

    pub fn set_stage(
        &mut self,
        stage: BakeStage,
        message: impl Into<String>,
    ) -> Result<(), MapCacheError> {
        if self.is_cancelled() {
            return Err(MapCacheError::Cancelled);
        }
        self.checkpoint.stage = stage;
        self.checkpoint_dirty = true;
        self.persist_checkpoint()?;
        self.report(stage_to_progress(stage), message.into());
        Ok(())
    }

    /// Write one complete output unit and checkpoint it.  A second invocation
    /// for a verified completed unit performs no write and no raster work.
    pub fn publish_unit(&mut self, key: UnitKey, payload: &[u8]) -> Result<bool, MapCacheError> {
        if self.is_cancelled() {
            if self.checkpoint_dirty {
                self.persist_checkpoint()?;
            }
            return Err(MapCacheError::Cancelled);
        }
        if self.should_skip(key) {
            return Ok(false);
        }
        self.validate_unit(key, payload)?;
        let previous = self
            .checkpoint
            .completed
            .as_slice()
            .last()
            .map(|unit| unit.key);
        if previous.is_some_and(|previous| previous >= key) {
            return Err(MapCacheError::Map(MapError::Invalid(
                "checkpoint unit ordering",
            )));
        }
        let next_bytes = self
            .bytes
            .checked_add(payload.len() as u64)
            .ok_or(MapCacheError::Map(MapError::Limit("artifact bytes")))?;
        if next_bytes > max_artifact_bytes(self.artifact()) {
            return Err(MapCacheError::Map(MapError::Limit("artifact bytes")));
        }
        let payload_bytes = payload.len() as u64;
        if self.cache_bytes.saturating_add(payload_bytes) > MAX_GENERATED_CACHE_BYTES {
            self.root
                .ensure_capacity(payload_bytes, &self.required, Some(&self.cancel))?;
            self.cache_bytes = self.root.generated_bytes()?;
        }
        let relative = unit_relative_path(key)?;
        atomic_write(&self.partial.join(relative), payload)?;
        self.cache_bytes = self.cache_bytes.saturating_add(payload_bytes);
        let mut completed = self.checkpoint.completed.as_slice().to_vec();
        completed.push(CompletedUnit {
            key,
            payload: PayloadReceipt {
                bytes: payload.len() as u32,
                sha256: Digest::of(payload),
            },
        });
        self.checkpoint.completed = nav::map::Rows::new(completed)?;
        self.bytes = next_bytes;
        self.checkpoint_dirty = true;
        if self
            .checkpoint
            .completed
            .as_slice()
            .len()
            .is_multiple_of(CHECKPOINT_INTERVAL)
        {
            self.persist_checkpoint()?;
        }
        self.report(
            stage_to_progress(self.checkpoint.stage),
            format!(
                "completed {}/{} units",
                self.checkpoint.completed.as_slice().len(),
                self.checkpoint.planned_units
            ),
        );
        Ok(true)
    }

    fn validate_unit(&self, key: UnitKey, payload: &[u8]) -> Result<(), MapCacheError> {
        match (self.identity, key) {
            (ArtifactIdentity::Catalogue(identity), UnitKey::ClientPois) => {
                let pois = ClientPois::decode(payload, identity)?;
                let _ = pois;
                if payload.len() > MAX_JSON_BYTES {
                    return Err(MapCacheError::Map(MapError::Limit("catalogue payload")));
                }
            }
            (ArtifactIdentity::Image(_), UnitKey::Terrain { tile }) => {
                TileReceipt {
                    key: tile,
                    payload: PayloadReceipt {
                        bytes: payload.len() as u32,
                        sha256: Digest::of(payload),
                    },
                }
                .verify_png(payload)?;
            }
            _ => return Err(MapCacheError::Map(MapError::Invalid("artifact unit"))),
        }
        Ok(())
    }

    fn persist_checkpoint(&mut self) -> Result<(), MapCacheError> {
        let bytes = self.checkpoint.encode()?;
        let new_bytes = bytes.len() as u64;
        atomic_write(&self.partial.join("checkpoint.json"), &bytes)?;
        self.cache_bytes = self
            .cache_bytes
            .saturating_sub(self.checkpoint_bytes)
            .saturating_add(new_bytes);
        self.checkpoint_bytes = new_bytes;
        self.checkpoint_dirty = false;
        Ok(())
    }

    fn report(&self, stage: MapStage, message: String) {
        self.report_progress(
            stage,
            self.checkpoint.completed.as_slice().len() as u32,
            self.checkpoint.planned_units,
            message,
        );
    }

    /// Producer-facing stage update. `completed`/`total` are the raster
    /// units, which may lead the published checkpoint during a neighborhood bake.
    pub fn report_progress(
        &self,
        stage: MapStage,
        completed: u32,
        total: u32,
        message: impl Into<String>,
    ) {
        (self.progress)(MapProgress {
            stage,
            completed,
            total,
            message: message.into(),
        });
    }

    fn finish(mut self, output: BakeOutput) -> Result<PathBuf, MapCacheError> {
        if self.is_cancelled() {
            if self.checkpoint_dirty {
                self.persist_checkpoint()?;
            }
            return Err(MapCacheError::Cancelled);
        }
        if self.checkpoint.completed.as_slice().len() != self.checkpoint.planned_units as usize {
            if self.checkpoint_dirty {
                self.persist_checkpoint()?;
            }
            return Err(MapCacheError::Map(MapError::Invalid(
                "incomplete map artifact",
            )));
        }
        self.checkpoint.stage = BakeStage::Publishing;
        self.checkpoint_dirty = true;
        self.persist_checkpoint()?;
        self.report(MapStage::Publishing, "writing completion receipt".into());
        let manifest = match (self.identity, output) {
            (ArtifactIdentity::Catalogue(identity), BakeOutput::Catalogue(mut manifest)) => {
                if manifest.identity != identity {
                    return Err(MapCacheError::Map(MapError::Identity));
                }
                manifest.payload =
                    read_receipt(&self.partial.join("client-pois.json"), MAX_JSON_BYTES)?;
                let bytes = encode_json_bounded(&manifest)?;
                let checked = CatalogueManifest::decode(&bytes, identity)?;
                let payload = read_bounded(&self.partial.join("client-pois.json"), MAX_JSON_BYTES)?;
                checked.read_payload(&payload)?;
                bytes
            }
            (ArtifactIdentity::Image(identity), BakeOutput::Images(manifest)) => {
                if manifest.identity != identity {
                    return Err(MapCacheError::Map(MapError::Identity));
                }
                manifest.encode()?
            }
            _ => return Err(MapCacheError::Map(MapError::Invalid("artifact manifest"))),
        };
        atomic_write(&self.partial.join("manifest.json"), &manifest)?;
        sync_directory(&self.partial)?;
        Ok(self.partial)
    }
}

fn encode_json_bounded<T: Serialize>(value: &T) -> Result<Vec<u8>, MapCacheError> {
    let bytes = serde_json::to_vec(value).map_err(MapError::Schema)?;
    if bytes.len() > MAX_JSON_BYTES {
        return Err(MapCacheError::Map(MapError::Limit("manifest bytes")));
    }
    Ok(bytes)
}

fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>, MapCacheError> {
    let file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(MapCacheError::Map(MapError::Path));
    }
    if metadata.len() > limit as u64 {
        return Err(MapCacheError::Map(MapError::Limit("payload bytes")));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(MapCacheError::Map(MapError::Limit("payload bytes")));
    }
    Ok(bytes)
}

fn clear_checkpoint(directory: &Path) -> Result<(), MapCacheError> {
    match fs::remove_file(directory.join("checkpoint.json")) {
        Ok(()) => sync_directory(directory),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn read_receipt(path: &Path, limit: usize) -> Result<PayloadReceipt, MapCacheError> {
    let bytes = read_bounded(path, limit)?;
    Ok(PayloadReceipt {
        bytes: bytes.len() as u32,
        sha256: Digest::of(&bytes),
    })
}

fn unit_relative_path(key: UnitKey) -> Result<PathBuf, MapCacheError> {
    match key {
        UnitKey::ClientPois => Ok(PathBuf::from("client-pois.json")),
        UnitKey::Terrain { tile } => Ok(tile.relative_path()?),
    }
}

fn max_artifact_bytes(kind: ArtifactKind) -> u64 {
    match kind {
        ArtifactKind::Catalogue => MAX_CATALOGUE_KEY_BYTES,
        ArtifactKind::Images => MAX_IMAGE_KEY_BYTES.min(MAX_IMAGE_BYTES),
    }
}

fn stage_to_progress(stage: BakeStage) -> MapStage {
    match stage {
        BakeStage::Catalogue => MapStage::DerivingPois,
        BakeStage::BaseTerrain => MapStage::BakingPlane,
        BakeStage::Downsample => MapStage::BuildingZoomLevels,
        BakeStage::Publishing => MapStage::Publishing,
    }
}

/// Cross-process advisory lock guard. The OS releases this lock when the
/// owning process exits, so there is no PID marker or stale-lock takeover.
#[derive(Debug)]
struct PublishLock {
    file: File,
}

impl PublishLock {
    fn try_acquire(path: &Path) -> Result<Option<Self>, MapCacheError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Self { file })),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(error)) => Err(error.into()),
        }
    }

    fn try_acquire_shared(path: &Path) -> Result<Option<Self>, MapCacheError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        match file.try_lock_shared() {
            Ok(()) => Ok(Some(Self { file })),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(error)) => Err(error.into()),
        }
    }

    fn acquire(path: &Path, cancel: Option<&AtomicBool>) -> Result<Self, MapCacheError> {
        loop {
            if let Some(lock) = Self::try_acquire(path)? {
                return Ok(lock);
            }
            if cancel.is_some_and(|cancel| cancel.load(AtomicOrdering::Acquire)) {
                return Err(MapCacheError::Cancelled);
            }
            thread::sleep(LOCK_RETRY);
        }
    }
}

impl Drop for PublishLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), MapCacheError> {
    let parent = path
        .parent()
        .ok_or_else(|| MapCacheError::Message("map artifact has no parent".into()))?;
    fs::create_dir_all(parent)?;
    let counter = TEMP_COUNTER.fetch_add(1, AtomicOrdering::Relaxed);
    let name = path
        .file_name()
        .ok_or_else(|| MapCacheError::Message("map artifact has no filename".into()))?
        .to_string_lossy();
    let temp = parent.join(format!(".{name}.tmp-{}-{counter}", std::process::id()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .truncate(false)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        Ok::<(), io::Error>(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.map_err(MapCacheError::Io)
}

#[cfg(not(windows))]
fn sync_directory(path: &Path) -> Result<(), MapCacheError> {
    match File::open(path).and_then(|file| file.sync_all()) {
        Ok(()) => Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::Unsupported | io::ErrorKind::InvalidInput
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(windows)]
fn sync_directory(_path: &Path) -> Result<(), MapCacheError> {
    // Rust's std directory open lacks FILE_FLAG_BACKUP_SEMANTICS; the file
    // writes are synced individually, so there is no portable directory sync.
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct JobKey {
    image: Digest,
    catalogue: Digest,
}

struct ActiveJob {
    key: JobKey,
    generation: u64,
    demand: Arc<AtomicUsize>,
    consumers: Arc<AtomicUsize>,
    cancel: Arc<AtomicBool>,
    status: Arc<Mutex<MapJobStatus>>,
    ready: Arc<Mutex<Option<ReadyMap>>>,
    join: Option<JoinHandle<()>>,
}
struct JobThreadState {
    demand: Arc<AtomicUsize>,
    consumers: Arc<AtomicUsize>,
    cancel: Arc<AtomicBool>,
    status: Arc<Mutex<MapJobStatus>>,
    ready: Arc<Mutex<Option<ReadyMap>>>,
}
struct ReadyCache {
    catalogues: BTreeMap<Digest, Weak<ReadyCatalogue>>,
    images: BTreeMap<Digest, Weak<ReadyImages>>,
}

struct ManagerInner {
    root: MapCacheRoot,
    producer: Arc<dyn MapBakeProducer>,
    active: Mutex<Option<ActiveJob>>,
    ready: Mutex<ReadyCache>,
    next_generation: AtomicU64,
}

/// One application-level map worker.  Clone this owner for panel/TUI callers;
/// the owner remains shared and no bot receives a map copy.
#[derive(Clone)]
pub struct MapDemandManager {
    inner: Arc<ManagerInner>,
}

impl fmt::Debug for MapDemandManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MapDemandManager")
            .field("root", &self.inner.root)
            .finish_non_exhaustive()
    }
}

impl MapDemandManager {
    pub fn new(root: MapCacheRoot, producer: Arc<dyn MapBakeProducer>) -> Self {
        Self {
            inner: Arc::new(ManagerInner {
                root,
                producer,
                active: Mutex::new(None),
                next_generation: AtomicU64::new(1),
                ready: Mutex::new(ReadyCache {
                    catalogues: BTreeMap::new(),
                    images: BTreeMap::new(),
                }),
            }),
        }
    }

    pub fn root(&self) -> &MapCacheRoot {
        &self.inner.root
    }

    pub fn request(
        &self,
        descriptor: MapProfileDescriptor,
        demand: MapDemand,
    ) -> Result<MapDemandHandle, MapCacheError> {
        let key = JobKey {
            image: descriptor.image_key()?,
            catalogue: descriptor.catalogue_key()?,
        };
        let handle_descriptor = descriptor.clone().without_input();
        // One lookup both probes and leases the ready set.  A key that another
        // process holds exclusively or prunes meanwhile is simply not ready,
        // and the worker below waits for it; it is never a request error.
        if let Some(ready) = self.lookup_ready(&descriptor, demand)? {
            let ready = Arc::new(ready);
            return Ok(MapDemandHandle::already_ready(
                self.clone(),
                handle_descriptor.clone(),
                demand,
                ready,
            ));
        }
        let mut active = self.inner.active.lock();
        if let Some(job) = (*active).as_mut() {
            if job.key == key {
                if demand == MapDemand::Images {
                    job.demand.store(1, AtomicOrdering::Release);
                }
                let status = job.status.lock().clone();
                if matches!(status, MapJobStatus::Failed(_)) {
                    return Ok(MapDemandHandle::terminal(
                        self.clone(),
                        handle_descriptor.clone(),
                        demand,
                        status,
                    ));
                }
                if !job.cancel.load(AtomicOrdering::Acquire) && !is_terminal(&status) {
                    job.consumers.fetch_add(1, AtomicOrdering::AcqRel);
                    return Ok(MapDemandHandle::running(
                        self.clone(),
                        handle_descriptor.clone(),
                        demand,
                        job.generation,
                        Arc::clone(&job.status),
                        Arc::clone(&job.ready),
                    ));
                }
            }
        }
        let predecessor = if let Some(job) = (*active).take() {
            if !is_terminal(&job.status.lock()) {
                job.cancel.store(true, AtomicOrdering::Release);
            }
            job.join
        } else {
            None
        };
        let generation = self
            .inner
            .next_generation
            .fetch_add(1, AtomicOrdering::Relaxed);
        let demand_flag = Arc::new(AtomicUsize::new(usize::from(demand == MapDemand::Images)));
        let consumers = Arc::new(AtomicUsize::new(1));
        let ready_slot = Arc::new(Mutex::new(None));
        let cancel = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new(MapJobStatus::Queued));
        let thread_state = JobThreadState {
            demand: Arc::clone(&demand_flag),
            consumers: Arc::clone(&consumers),
            cancel: Arc::clone(&cancel),
            status: Arc::clone(&status),
            ready: Arc::clone(&ready_slot),
        };
        let thread_root = self.inner.root.clone();
        let thread_producer = Arc::clone(&self.inner.producer);
        let thread_descriptor = descriptor.clone();
        let thread_predecessor = predecessor;
        let join = thread::Builder::new()
            .name("274bot-map-bake".into())
            .spawn(move || {
                if let Some(predecessor) = thread_predecessor {
                    let _ = predecessor.join();
                }
                run_job(
                    thread_root,
                    thread_producer,
                    thread_descriptor,
                    thread_state,
                )
            })
            .map_err(MapCacheError::Io)?;
        *active = Some(ActiveJob {
            key,
            generation,
            demand: demand_flag,
            consumers,
            cancel,
            status: Arc::clone(&status),
            ready: Arc::clone(&ready_slot),
            join: Some(join),
        });
        Ok(MapDemandHandle::running(
            self.clone(),
            handle_descriptor,
            demand,
            generation,
            status,
            ready_slot,
        ))
    }
    pub fn request_catalogue(
        &self,
        descriptor: MapProfileDescriptor,
    ) -> Result<MapDemandHandle, MapCacheError> {
        self.request(descriptor, MapDemand::CatalogueOnly)
    }

    pub fn request_images(
        &self,
        descriptor: MapProfileDescriptor,
    ) -> Result<MapDemandHandle, MapCacheError> {
        self.request(descriptor, MapDemand::Images)
    }

    /// Explicitly retry a failed/paused identity.  Ordinary `request` does
    /// not turn a failed worker into an automatic open/retry loop.
    pub fn retry(
        &self,
        descriptor: MapProfileDescriptor,
        demand: MapDemand,
    ) -> Result<MapDemandHandle, MapCacheError> {
        let key = JobKey {
            image: descriptor.image_key()?,
            catalogue: descriptor.catalogue_key()?,
        };
        let mut active = self.inner.active.lock();
        let reset = (*active)
            .as_ref()
            .is_some_and(|job| job.key == key && is_terminal(&job.status.lock()));
        if reset {
            let _ = (*active).take();
        }
        drop(active);
        self.request(descriptor, demand)
    }

    fn cached_catalogue(
        &self,
        identity: CatalogueIdentity,
    ) -> Result<Option<Arc<ReadyCatalogue>>, MapCacheError> {
        let key = identity.key()?.0;
        let path = self.inner.root.catalogue_dir(identity)?;
        let mut cache = self.inner.ready.lock();
        if !real_directory(&path)? {
            cache.catalogues.remove(&key);
            return Ok(None);
        }
        if let Some(value) = cache.catalogues.get(&key).and_then(Weak::upgrade) {
            return Ok(Some(value));
        }
        let Some(value) = self.inner.root.open_catalogue(identity)? else {
            cache.catalogues.remove(&key);
            return Ok(None);
        };
        cache.catalogues.insert(key, Arc::downgrade(&value));
        Ok(Some(value))
    }

    fn cached_images(
        &self,
        identity: ImageIdentity,
    ) -> Result<Option<Arc<ReadyImages>>, MapCacheError> {
        let key = identity.key()?.0;
        let path = self.inner.root.image_dir(identity)?;
        let mut cache = self.inner.ready.lock();
        if !real_directory(&path)? {
            cache.images.remove(&key);
            return Ok(None);
        }
        if let Some(value) = cache.images.get(&key).and_then(Weak::upgrade) {
            return Ok(Some(value));
        }
        let Some(value) = self.inner.root.open_images(identity)? else {
            cache.images.remove(&key);
            return Ok(None);
        };
        cache.images.insert(key, Arc::downgrade(&value));
        Ok(Some(value))
    }

    fn lookup_ready(
        &self,
        descriptor: &MapProfileDescriptor,
        demand: MapDemand,
    ) -> Result<Option<ReadyMap>, MapCacheError> {
        let Some(catalogue) = self.cached_catalogue(descriptor.catalogue_identity())? else {
            return Ok(None);
        };
        let images = if demand == MapDemand::Images {
            let Some(images) = self.cached_images(descriptor.image_identity())? else {
                return Ok(None);
            };
            Some(images)
        } else {
            None
        };
        Ok(Some(ReadyMap { catalogue, images }))
    }

    pub fn ready_for(
        &self,
        descriptor: &MapProfileDescriptor,
        demand: MapDemand,
    ) -> Result<bool, MapCacheError> {
        Ok(self.lookup_ready(descriptor, demand)?.is_some())
    }

    pub fn open_ready(
        &self,
        descriptor: &MapProfileDescriptor,
        demand: MapDemand,
    ) -> Result<ReadyMap, MapCacheError> {
        let catalogue = self
            .cached_catalogue(descriptor.catalogue_identity())?
            .ok_or(MapCacheError::Message("catalogue is not ready".into()))?;
        let images = if demand == MapDemand::Images {
            Some(
                self.cached_images(descriptor.image_identity())?
                    .ok_or(MapCacheError::Message("images are not ready".into()))?,
            )
        } else {
            None
        };
        Ok(ReadyMap { catalogue, images })
    }

    fn release(&self, generation: u64) {
        let mut active = self.inner.active.lock();
        let Some(job) = (*active).as_mut() else {
            return;
        };
        if job.generation != generation {
            return;
        }
        if job.consumers.fetch_sub(1, AtomicOrdering::AcqRel) == 1 {
            job.ready.lock().take();
            if !is_terminal(&job.status.lock()) {
                job.cancel.store(true, AtomicOrdering::Release);
            }
        }
    }

    /// Reap a finished worker without touching ready artefacts.  This is safe
    /// to call from a UI cadence and never scans the generated cache.
    pub fn reap(&self) {
        let mut active = self.inner.active.lock();
        let finished = (*active)
            .as_ref()
            .and_then(|job| job.join.as_ref())
            .is_some_and(JoinHandle::is_finished);
        if finished {
            if let Some(job) = (*active).as_mut() {
                if let Some(join) = job.join.take() {
                    let _ = join.join();
                }
            }
        }
    }
}

fn is_terminal(status: &MapJobStatus) -> bool {
    matches!(
        status,
        MapJobStatus::Ready
            | MapJobStatus::Paused
            | MapJobStatus::Cancelled
            | MapJobStatus::Failed(_)
    )
}

fn retain_ready(
    slot: &Arc<Mutex<Option<ReadyMap>>>,
    consumers: &Arc<AtomicUsize>,
    ready: ReadyMap,
) {
    let mut slot = slot.lock();
    if consumers.load(AtomicOrdering::Acquire) != 0 {
        *slot = Some(ready);
    }
}

fn run_job(
    root: MapCacheRoot,
    producer: Arc<dyn MapBakeProducer>,
    descriptor: MapProfileDescriptor,
    state: JobThreadState,
) {
    let JobThreadState {
        demand,
        consumers,
        cancel,
        status,
        ready,
    } = state;
    let progress = {
        let status = Arc::clone(&status);
        Arc::new(move |progress: MapProgress| {
            *status.lock() = MapJobStatus::Running(progress);
        }) as Arc<dyn Fn(MapProgress) + Send + Sync>
    };
    let result = (|| {
        let identity = descriptor.catalogue_identity();
        let lease = execute_artifact(
            &root,
            &producer,
            &descriptor,
            ArtifactKind::Catalogue,
            Arc::clone(&cancel),
            Arc::clone(&progress),
        )?;
        // This shared lease keeps the catalogue from being pruned during the
        // image stage without excluding any other reader.
        let catalogue = Arc::new(ReadyCatalogue::open(
            &root.catalogue_dir(identity)?,
            identity,
            lease,
        )?);
        if demand.load(AtomicOrdering::Acquire) != 1 {
            return Ok(ReadyMap {
                catalogue,
                images: None,
            });
        }
        let identity = descriptor.image_identity();
        let lease = execute_artifact(
            &root,
            &producer,
            &descriptor,
            ArtifactKind::Images,
            Arc::clone(&cancel),
            Arc::clone(&progress),
        )?;
        let images = ReadyImages::open(&root.image_dir(identity)?, identity, lease)?;
        Ok(ReadyMap {
            catalogue,
            images: Some(Arc::new(images)),
        })
    })();
    match result {
        Ok(ready_map) => {
            retain_ready(&ready, &consumers, ready_map);
            *status.lock() = MapJobStatus::Ready;
        }
        Err(error) => finish_job(error, &status, &cancel),
    }
}

fn finish_job(error: MapCacheError, status: &Arc<Mutex<MapJobStatus>>, cancel: &Arc<AtomicBool>) {
    let terminal =
        if matches!(error, MapCacheError::Cancelled) || cancel.load(AtomicOrdering::Acquire) {
            MapJobStatus::Paused
        } else {
            MapJobStatus::Failed(error.to_string())
        };
    *status.lock() = terminal;
}

/// Bakes `artifact` unless it is already published and returns a shared lease
/// on its ready directory.
///
/// The exclusive per-key lock is held only while baking and publishing, and it
/// is dropped before the lease is taken on a fresh handle.  Re-locking the
/// exclusive handle shared is not a downgrade on Windows (LockFileEx keeps both
/// locks), and a worker that kept an exclusive lock would exclude every other
/// reader.  If a prune removes the directory between the two locks, the next
/// pass finds it not ready and bakes again.
fn execute_artifact(
    root: &MapCacheRoot,
    producer: &Arc<dyn MapBakeProducer>,
    descriptor: &MapProfileDescriptor,
    artifact: ArtifactKind,
    cancel: Arc<AtomicBool>,
    progress: Arc<dyn Fn(MapProgress) + Send + Sync>,
) -> Result<PublishLock, MapCacheError> {
    let key = match artifact {
        ArtifactKind::Catalogue => descriptor.catalogue_key()?,
        ArtifactKind::Images => descriptor.image_key()?,
    };
    let lock_path = root.lock_path(descriptor.revision(), artifact, key);
    let ready = match artifact {
        ArtifactKind::Catalogue => root.catalogue_dir(descriptor.catalogue_identity())?,
        ArtifactKind::Images => root.image_dir(descriptor.image_identity())?,
    };
    root.ensure_kind_dir(descriptor.revision(), artifact)?;
    let mut required = BTreeSet::new();
    if artifact == ArtifactKind::Images {
        required.insert(root.catalogue_dir(descriptor.catalogue_identity())?);
    }
    required.insert(root.partial_dir(descriptor.revision(), artifact, key));
    let mut capacity_checked = false;
    loop {
        if let Some(lease) = ready_lease(&lock_path, &ready)? {
            clear_checkpoint(&ready)?;
            return Ok(lease);
        }
        if cancel.load(AtomicOrdering::Acquire) {
            return Err(MapCacheError::Cancelled);
        }
        if !capacity_checked {
            // Establish the global-capacity state before contending for the
            // per-key lock.  Later capacity checks under the per-key lock only
            // try-lock entries, so no capacity/per-key wait cycle forms.
            root.ensure_capacity(0, &required, Some(&cancel))?;
            capacity_checked = true;
        }
        let Some(lock) = PublishLock::try_acquire(&lock_path)? else {
            // Another publisher or pruner owns the key.  Poll readiness again
            // instead of waiting for an exclusive lock that readers' shared
            // leases may block indefinitely.
            thread::sleep(LOCK_RETRY);
            continue;
        };
        if real_directory(&ready)? {
            continue;
        }
        let request = BakeRequest {
            artifact,
            descriptor: descriptor.clone(),
        };
        let plan = producer.plan(&request)?.checked()?;
        let mut writer = BakeWriter::open(
            root.clone(),
            &request,
            plan,
            Arc::clone(&cancel),
            Arc::clone(&progress),
            required.clone(),
        )?;
        if artifact == ArtifactKind::Catalogue {
            writer.set_stage(BakeStage::Catalogue, "deriving client POIs")?;
        } else {
            writer.set_stage(BakeStage::BaseTerrain, "baking terrain tiles")?;
        }
        let output = match producer.run(&request, &mut writer) {
            Ok(output) => output,
            Err(error) => {
                let _ = writer.persist_checkpoint();
                return Err(error);
            }
        };
        let partial = writer.finish(output)?;
        fs::rename(partial, &ready)?;
        sync_directory(ready.parent().unwrap())?;
        clear_checkpoint(&ready)?;
        drop(lock);
    }
}

/// Ready immutable handles shared by both UI frontends.  The image handle is
/// optional for a TUI catalogue-only demand.
#[derive(Clone)]
pub struct ReadyMap {
    pub catalogue: Arc<ReadyCatalogue>,
    pub images: Option<Arc<ReadyImages>>,
}

impl fmt::Debug for ReadyMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReadyMap")
            .field("catalogue", &self.catalogue.manifest())
            .field("has_images", &self.images.is_some())
            .finish()
    }
}

struct ConsumerLease {
    manager: Arc<ManagerInner>,
    generation: u64,
    released: AtomicBool,
}
impl ConsumerLease {
    fn release(&self) {
        if !self.released.swap(true, AtomicOrdering::AcqRel) {
            MapDemandManager {
                inner: Arc::clone(&self.manager),
            }
            .release(self.generation);
        }
    }
}
impl Drop for ConsumerLease {
    fn drop(&mut self) {
        self.release();
    }
}

/// Handle returned by an explicit panel/TUI demand.  Dropping the last active
/// handle parks the worker at its latest validated checkpoint; it does not
/// delete completed units or automatically retry a failure.
pub struct MapDemandHandle {
    manager: MapDemandManager,
    descriptor: MapProfileDescriptor,
    demand: MapDemand,
    generation: Option<u64>,
    status: Arc<Mutex<MapJobStatus>>,
    ready_slot: Option<Arc<Mutex<Option<ReadyMap>>>>,
    ready_owner: Option<Arc<ReadyMap>>,
    lease: Option<Arc<ConsumerLease>>,
}

impl fmt::Debug for MapDemandHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MapDemandHandle")
            .field("demand", &self.demand)
            .field("generation", &self.generation)
            .field("status", &self.status())
            .finish()
    }
}

impl MapDemandHandle {
    fn running(
        manager: MapDemandManager,
        descriptor: MapProfileDescriptor,
        demand: MapDemand,
        generation: u64,
        status: Arc<Mutex<MapJobStatus>>,
        ready_slot: Arc<Mutex<Option<ReadyMap>>>,
    ) -> Self {
        let lease = Arc::new(ConsumerLease {
            manager: Arc::clone(&manager.inner),
            generation,
            released: AtomicBool::new(false),
        });
        Self {
            manager,
            descriptor,
            demand,
            generation: Some(generation),
            status,
            ready_slot: Some(ready_slot),
            ready_owner: None,
            lease: Some(lease),
        }
    }

    fn already_ready(
        manager: MapDemandManager,
        descriptor: MapProfileDescriptor,
        demand: MapDemand,
        ready: Arc<ReadyMap>,
    ) -> Self {
        Self {
            manager,
            descriptor,
            demand,
            generation: None,
            status: Arc::new(Mutex::new(MapJobStatus::Ready)),
            ready_slot: None,
            ready_owner: Some(ready),
            lease: None,
        }
    }
    fn terminal(
        manager: MapDemandManager,
        descriptor: MapProfileDescriptor,
        demand: MapDemand,
        status: MapJobStatus,
    ) -> Self {
        Self {
            manager,
            descriptor,
            demand,
            generation: None,
            status: Arc::new(Mutex::new(status)),
            ready_slot: None,
            ready_owner: None,
            lease: None,
        }
    }

    pub fn demand(&self) -> MapDemand {
        self.demand
    }
    pub fn generation(&self) -> Option<u64> {
        self.generation
    }
    pub fn status(&self) -> MapJobStatus {
        let status = self.status.lock().clone();
        if is_terminal(&status) {
            self.manager.reap();
        }
        status
    }
    pub fn cancel(&self) {
        if let Some(lease) = &self.lease {
            lease.release();
        }
    }
    pub fn ready(&self) -> Result<ReadyMap, MapCacheError> {
        if let Some(owner) = &self.ready_owner {
            return Ok((**owner).clone());
        }
        if let Some(slot) = &self.ready_slot {
            if let Some(ready) = slot.lock().as_ref() {
                return Ok(ready.clone());
            }
        }
        self.manager.open_ready(&self.descriptor, self.demand)
    }
    pub fn descriptor(&self) -> &MapProfileDescriptor {
        &self.descriptor
    }
}

impl Clone for MapDemandHandle {
    fn clone(&self) -> Self {
        if let Some(lease) = &self.lease {
            // A cloned UI view shares one cancellation lease.  The original
            // and clone therefore cannot accidentally cancel each other twice.
            Self {
                manager: self.manager.clone(),
                descriptor: self.descriptor.clone(),
                demand: self.demand,
                generation: self.generation,
                status: Arc::clone(&self.status),
                ready_slot: self.ready_slot.as_ref().map(Arc::clone),
                ready_owner: self.ready_owner.as_ref().map(Arc::clone),
                lease: Some(Arc::clone(lease)),
            }
        } else if let Some(owner) = &self.ready_owner {
            Self::already_ready(
                self.manager.clone(),
                self.descriptor.clone(),
                self.demand,
                Arc::clone(owner),
            )
        } else {
            Self::terminal(
                self.manager.clone(),
                self.descriptor.clone(),
                self.demand,
                self.status(),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nav::map::formats::{ColorFormat, Coverage, CoverageLevel, ImageManifest, PlaneBounds};
    use nav::map::identity::{CATALOGUE_SCHEMA, IMAGE_SCHEMA};
    use nav::map::poi::PoiRecord;
    use nav::map::spatial::{TileKey, WorldBounds, TILE_GUTTER, TILE_INTERIOR};
    use std::io::{BufRead, BufReader, Read};
    use std::process::{Command, Stdio};
    use std::sync::atomic::AtomicUsize;
    use std::time::Instant;

    fn temp_root(name: &str) -> MapCacheRoot {
        let path =
            std::env::temp_dir().join(format!("274bot-map-cache-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        MapCacheRoot::from_root(path)
    }

    fn descriptor(root: &MapCacheRoot) -> MapProfileDescriptor {
        let _ = root;
        MapProfileDescriptor::new(274, Digest([1; 32]), Digest([2; 32]), Digest([3; 32])).unwrap()
    }

    struct FixtureProducer {
        rasterized: Arc<AtomicUsize>,
        runs: AtomicUsize,
        image_started: AtomicBool,
        image_exited: Arc<AtomicBool>,
        image_delay_ms: AtomicU64,
        hold_images: AtomicBool,
        interrupt_once: AtomicBool,
    }

    struct ImageExit(Arc<AtomicBool>);

    impl Drop for ImageExit {
        fn drop(&mut self) {
            self.0.store(true, AtomicOrdering::Release);
        }
    }

    impl FixtureProducer {
        fn new(interrupt_once: bool) -> Arc<Self> {
            Arc::new(Self {
                rasterized: Arc::new(AtomicUsize::new(0)),
                runs: AtomicUsize::new(0),
                image_started: AtomicBool::new(false),
                image_exited: Arc::new(AtomicBool::new(false)),
                image_delay_ms: AtomicU64::new(0),
                hold_images: AtomicBool::new(false),
                interrupt_once: AtomicBool::new(interrupt_once),
            })
        }

        fn tile_keys() -> [TileKey; 2] {
            [
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
            ]
        }

        fn fixture_png(seed: u8) -> Vec<u8> {
            // Complete deterministic PNG: IHDR + a zlib stream containing
            // one filtered RGBA row per pixel.  The fixture is intentionally
            // small in policy (two units), but real enough for A's PNG guard
            // and ReadyImages reader; no mocked payload is published.
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
    }

    impl MapBakeProducer for FixtureProducer {
        fn plan(&self, request: &BakeRequest) -> Result<BakePlan, MapCacheError> {
            Ok(match request.artifact() {
                ArtifactKind::Catalogue => BakePlan {
                    planned_units: 1,
                    stage: BakeStage::Catalogue,
                },
                ArtifactKind::Images => BakePlan {
                    planned_units: 2,
                    stage: BakeStage::BaseTerrain,
                },
            })
        }

        fn run(
            &self,
            request: &BakeRequest,
            writer: &mut BakeWriter,
        ) -> Result<BakeOutput, MapCacheError> {
            self.runs.fetch_add(1, AtomicOrdering::Relaxed);
            match request.artifact() {
                ArtifactKind::Catalogue => {
                    // Catalogue production is also a real checked unit in this
                    // lifecycle test; the format-specific decoder is tested
                    // by production B and A's checked readers.
                    let expected = request.descriptor().catalogue_identity();
                    let payload = ClientPois {
                        schema: CATALOGUE_SCHEMA,
                        identity: expected,
                        coverage: Coverage {
                            npc_placements: CoverageLevel::Unavailable,
                            bank_services: CoverageLevel::Limited,
                            place_labels: CoverageLevel::Unavailable,
                            unresolved: nav::map::Rows::new(Vec::new())?,
                        },
                        records: nav::map::Rows::<PoiRecord, 4096>::new(Vec::new())?,
                    }
                    .encode()?;
                    writer.publish_unit(UnitKey::ClientPois, &payload)?;
                    Ok(BakeOutput::Catalogue(CatalogueManifest {
                        schema: CATALOGUE_SCHEMA,
                        identity: expected,
                        key: expected.key()?,
                        record_count: 0,
                        payload: PayloadReceipt {
                            bytes: 1,
                            sha256: Digest([0; 32]),
                        },
                    }))
                }
                ArtifactKind::Images => {
                    let _image_exit = ImageExit(Arc::clone(&self.image_exited));
                    self.image_started.store(true, AtomicOrdering::Release);
                    let delay = self.image_delay_ms.load(AtomicOrdering::Acquire);
                    if delay != 0 {
                        thread::sleep(Duration::from_millis(delay));
                    }
                    while self.hold_images.load(AtomicOrdering::Acquire) {
                        if writer.is_cancelled() {
                            return Err(MapCacheError::Cancelled);
                        }
                        thread::sleep(Duration::from_millis(1));
                    }
                    let keys = Self::tile_keys();
                    for (index, key) in keys.into_iter().enumerate() {
                        if writer.should_skip(UnitKey::Terrain { tile: key }) {
                            continue;
                        }
                        self.rasterized.fetch_add(1, AtomicOrdering::Relaxed);
                        writer.publish_unit(
                            UnitKey::Terrain { tile: key },
                            &Self::fixture_png(index as u8 + 1),
                        )?;
                        if self.interrupt_once.swap(false, AtomicOrdering::AcqRel) {
                            return Err(MapCacheError::Cancelled);
                        }
                    }
                    let identity = request.descriptor().image_identity();
                    let pngs = keys.map(|key| {
                        let path = writer
                            .partial
                            .join(unit_relative_path(UnitKey::Terrain { tile: key }).unwrap());
                        let bytes = fs::read(path).unwrap();
                        TileReceipt {
                            key,
                            payload: PayloadReceipt {
                                bytes: bytes.len() as u32,
                                sha256: Digest::of(&bytes),
                            },
                        }
                    });
                    Ok(BakeOutput::Images(ImageManifest {
                        schema: IMAGE_SCHEMA,
                        identity,
                        key: identity.key()?,
                        extent: WorldBounds {
                            west: 3200,
                            south: 3200,
                            east: 3328,
                            north: 3328,
                        },
                        planes: nav::map::Rows::new(vec![PlaneBounds {
                            plane: 0,
                            bounds: WorldBounds {
                                west: 3200,
                                south: 3200,
                                east: 3328,
                                north: 3328,
                            },
                        }])?,
                        max_lod: 1,
                        interior: TILE_INTERIOR,
                        gutter: TILE_GUTTER,
                        color: ColorFormat::Rgba8Unorm,
                        tiles: nav::map::Rows::new(pngs.to_vec())?,
                    }))
                }
            }
        }
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

    fn wait_ready(handle: &MapDemandHandle) {
        let start = Instant::now();
        while !matches!(handle.status(), MapJobStatus::Ready) {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "status: {:?}",
                handle.status()
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn fixture_interrupt_resume_and_restart_do_not_rasterize_ready_tiles() {
        let root = temp_root("resume");
        let producer = FixtureProducer::new(true);
        let manager = MapDemandManager::new(root.clone(), producer.clone());
        let descriptor = descriptor(&root);
        let first = manager
            .request(descriptor.clone(), MapDemand::Images)
            .unwrap();
        let start = Instant::now();
        while !matches!(first.status(), MapJobStatus::Paused) {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "status: {:?}",
                first.status()
            );
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(producer.rasterized.load(AtomicOrdering::Relaxed), 1);
        let resumed = manager
            .request(descriptor.clone(), MapDemand::Images)
            .unwrap();
        wait_ready(&resumed);
        assert_eq!(producer.rasterized.load(AtomicOrdering::Relaxed), 2);
        let ready = resumed.ready().unwrap();
        assert!(ready.images.is_some());
        let ready_again = resumed.ready().unwrap();
        assert!(Arc::ptr_eq(&ready.catalogue, &ready_again.catalogue));
        assert!(Arc::ptr_eq(
            ready.images.as_ref().unwrap(),
            ready_again.images.as_ref().unwrap()
        ));
        let image_dir = root.image_dir(descriptor.image_identity()).unwrap();
        assert!(!image_dir.join("checkpoint.json").exists());
        let manifest_modified = fs::metadata(image_dir.join("manifest.json"))
            .unwrap()
            .modified()
            .unwrap();
        let manifest_before = fs::read(image_dir.join("manifest.json")).unwrap();
        drop(resumed);
        manager.reap();

        // A fresh manager models process restart: ready metadata is opened
        // directly and no producer method is invoked.
        let restarted_producer = FixtureProducer::new(false);
        let restarted = MapDemandManager::new(root.clone(), restarted_producer.clone());
        let mut restarted_catalogue = None;
        let mut restarted_images = None;
        for _ in 0..20 {
            let reopened = restarted
                .request(descriptor.clone(), MapDemand::Images)
                .unwrap();
            assert!(matches!(reopened.status(), MapJobStatus::Ready));
            assert_eq!(
                restarted_producer.rasterized.load(AtomicOrdering::Relaxed),
                0
            );
            let reopened_map = reopened.ready().unwrap();
            if let Some(cached) = &restarted_catalogue {
                assert!(Arc::ptr_eq(cached, &reopened_map.catalogue));
            } else {
                restarted_catalogue = Some(Arc::clone(&reopened_map.catalogue));
            }
            if let Some(cached) = &restarted_images {
                assert!(Arc::ptr_eq(cached, reopened_map.images.as_ref().unwrap()));
            } else {
                restarted_images = Some(Arc::clone(reopened_map.images.as_ref().unwrap()));
            }
            drop(reopened_map);
            assert_eq!(
                manifest_modified,
                fs::metadata(image_dir.join("manifest.json"))
                    .unwrap()
                    .modified()
                    .unwrap()
            );
            assert_eq!(
                manifest_before,
                fs::read(image_dir.join("manifest.json")).unwrap()
            );
        }
        let _ = fs::remove_dir_all(root.path());
    }

    struct ResumeAdoptProducer {
        rasterized: Arc<AtomicUsize>,
        interrupt_once: AtomicBool,
    }

    impl ResumeAdoptProducer {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                rasterized: Arc::new(AtomicUsize::new(0)),
                interrupt_once: AtomicBool::new(true),
            })
        }

        fn keys() -> [nav::map::spatial::TileKey; 3] {
            [
                nav::map::spatial::TileKey {
                    plane: 0,
                    lod: 0,
                    x: 50,
                    z: 50,
                },
                nav::map::spatial::TileKey {
                    plane: 0,
                    lod: 0,
                    x: 51,
                    z: 50,
                },
                nav::map::spatial::TileKey {
                    plane: 0,
                    lod: 1,
                    x: 25,
                    z: 25,
                },
            ]
        }
    }

    impl MapBakeProducer for ResumeAdoptProducer {
        fn plan(&self, request: &BakeRequest) -> Result<BakePlan, MapCacheError> {
            Ok(match request.artifact() {
                ArtifactKind::Catalogue => BakePlan {
                    planned_units: 1,
                    stage: BakeStage::Catalogue,
                },
                ArtifactKind::Images => BakePlan {
                    planned_units: 3,
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
                ArtifactKind::Catalogue => FixtureProducer::new(false).run(request, writer),
                ArtifactKind::Images => {
                    let keys = Self::keys();
                    crate::map_producer::publish_existing_image_units(writer, &keys)?;
                    for (index, key) in keys.into_iter().enumerate() {
                        if writer.should_skip(UnitKey::Terrain { tile: key }) {
                            continue;
                        }
                        self.rasterized.fetch_add(1, AtomicOrdering::Relaxed);
                        writer.publish_unit(
                            UnitKey::Terrain { tile: key },
                            &FixtureProducer::fixture_png(index as u8 + 1),
                        )?;
                        if self.interrupt_once.swap(false, AtomicOrdering::AcqRel) {
                            return crate::map_producer::finish_image_bake(
                                writer,
                                nav::map::raster::ImageBakeOutcome::Paused(
                                    nav::map::raster::RasterMetrics {
                                        plan: nav::map::raster::PlanStats::default(),
                                        lods: Vec::new(),
                                        total_compressed_bytes: 0,
                                        peak_tracked_bytes: 0,
                                        elapsed: Duration::ZERO,
                                    },
                                ),
                            );
                        }
                    }
                    let identity = request.descriptor().image_identity();
                    let pngs = keys.map(|key| {
                        let path = writer
                            .directory()
                            .join(unit_relative_path(UnitKey::Terrain { tile: key }).unwrap());
                        let bytes = fs::read(path).unwrap();
                        TileReceipt {
                            key,
                            payload: PayloadReceipt {
                                bytes: bytes.len() as u32,
                                sha256: Digest::of(&bytes),
                            },
                        }
                    });
                    Ok(BakeOutput::Images(ImageManifest {
                        schema: IMAGE_SCHEMA,
                        identity,
                        key: identity.key()?,
                        extent: WorldBounds {
                            west: 3200,
                            south: 3200,
                            east: 3328,
                            north: 3328,
                        },
                        planes: nav::map::Rows::new(vec![PlaneBounds {
                            plane: 0,
                            bounds: WorldBounds {
                                west: 3200,
                                south: 3200,
                                east: 3328,
                                north: 3328,
                            },
                        }])?,
                        max_lod: 1,
                        interior: TILE_INTERIOR,
                        gutter: TILE_GUTTER,
                        color: ColorFormat::Rgba8Unorm,
                        tiles: nav::map::Rows::new(pngs.to_vec())?,
                    }))
                }
            }
        }
    }

    #[test]
    fn image_bake_interrupt_resumes_only_remaining_units() {
        let root = temp_root("unit-interrupt-resume");
        let descriptor = descriptor(&root);
        let keys = ResumeAdoptProducer::keys();
        let request = BakeRequest {
            artifact: ArtifactKind::Images,
            descriptor: descriptor.clone(),
        };
        let progress: Arc<dyn Fn(MapProgress) + Send + Sync> = Arc::new(|_| {});
        let writer = BakeWriter::open(
            root.clone(),
            &request,
            BakePlan {
                planned_units: 3,
                stage: BakeStage::BaseTerrain,
            },
            Arc::new(AtomicBool::new(false)),
            progress,
            BTreeSet::new(),
        )
        .unwrap();
        // Parent 23fe73df9 left rastered files uncheckpointed on close.
        let first = keys[0];
        let path = writer.directory().join(first.relative_path().unwrap());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, FixtureProducer::fixture_png(1)).unwrap();
        drop(writer);

        let producer = ResumeAdoptProducer::new();
        let manager = MapDemandManager::new(root.clone(), producer.clone());
        let first = manager
            .request(descriptor.clone(), MapDemand::Images)
            .unwrap();
        let start = Instant::now();
        while !matches!(first.status(), MapJobStatus::Paused) {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "status: {:?}",
                first.status()
            );
            thread::sleep(Duration::from_millis(5));
        }
        // Adopted the on-disk unit; rasterized only the next one before pause.
        assert_eq!(producer.rasterized.load(AtomicOrdering::Relaxed), 1);
        let resumed = manager.request(descriptor, MapDemand::Images).unwrap();
        wait_ready(&resumed);
        assert_eq!(
            producer.rasterized.load(AtomicOrdering::Relaxed),
            2,
            "resume must bake only the remaining unit"
        );
        let _ = fs::remove_dir_all(root.path());
    }

    #[test]
    fn joining_image_requests_share_one_bake() {
        let root = temp_root("join");
        let producer = FixtureProducer::new(false);
        producer.image_delay_ms.store(100, AtomicOrdering::Release);
        let manager = MapDemandManager::new(root.clone(), producer.clone());
        let descriptor = descriptor(&root);
        let first = manager
            .request(descriptor.clone(), MapDemand::Images)
            .unwrap();
        let start = Instant::now();
        while !producer.image_started.load(AtomicOrdering::Acquire) {
            assert!(start.elapsed() < Duration::from_secs(2));
            thread::sleep(Duration::from_millis(1));
        }
        let second = manager.request(descriptor, MapDemand::Images).unwrap();
        wait_ready(&first);
        wait_ready(&second);
        assert_eq!(producer.runs.load(AtomicOrdering::Relaxed), 2);
        drop(first);
        drop(second);
        let _ = fs::remove_dir_all(root.path());
    }

    #[test]
    fn catalogue_ready_lock_does_not_deadlock_images() {
        let root = temp_root("catalogue-lock");
        let producer = FixtureProducer::new(false);
        let manager = MapDemandManager::new(root.clone(), producer);
        let descriptor = descriptor(&root);
        let catalogue = manager
            .request(descriptor.clone(), MapDemand::CatalogueOnly)
            .unwrap();
        wait_ready(&catalogue);
        let catalogue_ready = catalogue.ready().unwrap();
        assert!(catalogue_ready.images.is_none());

        let images = manager.request(descriptor, MapDemand::Images).unwrap();
        wait_ready(&images);
        assert!(images.ready().unwrap().images.is_some());
        drop(catalogue_ready);
        drop(images);
        drop(catalogue);
    }

    #[test]
    fn ready_status_holds_publication_lease_until_handle_drop() {
        let root = temp_root("ready-lease");
        let producer = FixtureProducer::new(false);
        let manager = MapDemandManager::new(root.clone(), producer);
        let descriptor = descriptor(&root);
        let handle = manager
            .request(descriptor.clone(), MapDemand::Images)
            .unwrap();
        wait_ready(&handle);
        assert_eq!(root.prune_to(&BTreeSet::new(), 0).unwrap(), 0);
        assert!(handle.ready().unwrap().images.is_some());
        drop(handle);
        assert!(root.prune_to(&BTreeSet::new(), 0).unwrap() > 0);
    }

    #[test]
    fn dropping_last_handle_cancels_and_identity_switch_joins_off_ui() {
        let root = temp_root("switch");
        let producer = FixtureProducer::new(false);
        producer.image_delay_ms.store(200, AtomicOrdering::Release);
        let manager = MapDemandManager::new(root.clone(), producer.clone());
        let first_descriptor = descriptor(&root);
        let first = manager
            .request(first_descriptor, MapDemand::Images)
            .unwrap();
        let start = Instant::now();
        while !producer.image_started.load(AtomicOrdering::Acquire) {
            assert!(start.elapsed() < Duration::from_secs(2));
            thread::sleep(Duration::from_millis(1));
        }
        drop(first);
        let second_descriptor =
            MapProfileDescriptor::new(274, Digest([4; 32]), Digest([5; 32]), Digest([6; 32]))
                .unwrap();
        let request_start = Instant::now();
        let second = manager
            .request(second_descriptor.clone(), MapDemand::CatalogueOnly)
            .unwrap();
        assert!(request_start.elapsed() < Duration::from_millis(500));
        wait_ready(&second);
        assert!(producer.image_exited.load(AtomicOrdering::Acquire));
        assert!(root
            .catalogue_dir(second_descriptor.catalogue_identity())
            .unwrap()
            .is_dir());
        drop(second);
        let _ = fs::remove_dir_all(root.path());
    }

    #[test]
    fn checkpoint_is_removed_only_after_ready_rename() {
        let root = temp_root("checkpoint-boundary");
        let descriptor = descriptor(&root);
        let producer = FixtureProducer::new(false);
        let request = BakeRequest {
            artifact: ArtifactKind::Catalogue,
            descriptor: descriptor.clone(),
        };
        let progress: Arc<dyn Fn(MapProgress) + Send + Sync> = Arc::new(|_| {});
        let plan = producer.plan(&request).unwrap();
        let mut writer = BakeWriter::open(
            root.clone(),
            &request,
            plan,
            Arc::new(AtomicBool::new(false)),
            progress,
            BTreeSet::new(),
        )
        .unwrap();
        let output = producer.run(&request, &mut writer).unwrap();
        let partial = writer.finish(output).unwrap();
        assert!(partial
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".partial")));
        assert!(partial.join("manifest.json").is_file());
        assert!(partial.join("checkpoint.json").is_file());

        let ready = root.catalogue_dir(descriptor.catalogue_identity()).unwrap();
        fs::rename(&partial, &ready).unwrap();
        assert!(ready.join("checkpoint.json").exists());
        let manager = MapDemandManager::new(root.clone(), FixtureProducer::new(false));
        let reopened = manager
            .request(descriptor, MapDemand::CatalogueOnly)
            .unwrap();
        assert!(matches!(reopened.status(), MapJobStatus::Ready));
        assert!(!ready.join("checkpoint.json").exists());
        let _ = fs::remove_dir_all(root.path());
    }

    #[test]
    fn checkpoint_survives_crash_before_ready_rename() {
        let root = temp_root("checkpoint-before-rename");
        let descriptor = descriptor(&root);
        let producer = FixtureProducer::new(false);
        let manager = MapDemandManager::new(root.clone(), Arc::clone(&producer) as Arc<_>);
        let catalogue = manager
            .request(descriptor.clone(), MapDemand::CatalogueOnly)
            .unwrap();
        wait_ready(&catalogue);
        drop(catalogue);

        let request = BakeRequest {
            artifact: ArtifactKind::Images,
            descriptor: descriptor.clone(),
        };
        let progress: Arc<dyn Fn(MapProgress) + Send + Sync> = Arc::new(|_| {});
        let plan = producer.plan(&request).unwrap();
        let mut writer = BakeWriter::open(
            root.clone(),
            &request,
            plan,
            Arc::new(AtomicBool::new(false)),
            progress,
            BTreeSet::new(),
        )
        .unwrap();
        let checkpoint_before = fs::read(writer.partial.join("checkpoint.json")).unwrap();
        assert!(writer
            .publish_unit(
                UnitKey::Terrain {
                    tile: FixtureProducer::tile_keys()[0],
                },
                &FixtureProducer::fixture_png(1),
            )
            .unwrap());
        assert_eq!(
            checkpoint_before,
            fs::read(writer.partial.join("checkpoint.json")).unwrap()
        );
        let output = producer.run(&request, &mut writer).unwrap();
        let partial = writer.finish(output).unwrap();
        assert!(partial.join("manifest.json").is_file());
        assert!(partial.join("checkpoint.json").is_file());

        let resumed_producer = FixtureProducer::new(false);
        let resumed = MapDemandManager::new(root.clone(), resumed_producer.clone());
        let handle = resumed.request(descriptor, MapDemand::Images).unwrap();
        wait_ready(&handle);
        assert_eq!(resumed_producer.rasterized.load(AtomicOrdering::Relaxed), 0);
        let image_dir = root
            .image_dir(handle.descriptor().image_identity())
            .unwrap();
        assert!(!image_dir.join("checkpoint.json").exists());
    }
    #[test]
    fn client_input_requires_a_prepared_cache_lease() {
        let root = temp_root("client-input-missing");
        let request = BakeRequest {
            artifact: ArtifactKind::Catalogue,
            descriptor: descriptor(&root),
        };
        assert!(matches!(
            request.client_input(),
            Err(MapCacheError::NoPreparedCache)
        ));
    }
    #[test]
    fn bad_catalogue_receipt_is_rejected_before_ready_publication() {
        let root = temp_root("bad-receipt");
        let descriptor = descriptor(&root);
        let producer = FixtureProducer::new(false);
        let request = BakeRequest {
            artifact: ArtifactKind::Catalogue,
            descriptor: descriptor.clone(),
        };
        let progress: Arc<dyn Fn(MapProgress) + Send + Sync> = Arc::new(|_| {});
        let plan = producer.plan(&request).unwrap();
        let mut writer = BakeWriter::open(
            root.clone(),
            &request,
            plan,
            Arc::new(AtomicBool::new(false)),
            progress,
            BTreeSet::new(),
        )
        .unwrap();
        let output = match producer.run(&request, &mut writer).unwrap() {
            BakeOutput::Catalogue(mut manifest) => {
                manifest.record_count = 1;
                BakeOutput::Catalogue(manifest)
            }
            BakeOutput::Images(_) => panic!("fixture returned image output"),
        };
        match writer.finish(output) {
            Err(MapCacheError::Map(MapError::Invalid("POI receipt count"))) => {}
            Err(_) => panic!("bad catalogue receipt returned the wrong error"),
            Ok(_) => panic!("bad catalogue receipt was published"),
        }
        assert!(!root
            .catalogue_dir(descriptor.catalogue_identity())
            .unwrap()
            .exists());
        let _ = fs::remove_dir_all(root.path());
    }

    #[test]
    fn prune_respects_ready_leases_and_evicts_stale_partials() {
        let root = temp_root("prune-leased");
        let producer = FixtureProducer::new(false);
        let manager = MapDemandManager::new(root.clone(), producer);
        let descriptor = descriptor(&root);
        let handle = manager
            .request(descriptor.clone(), MapDemand::Images)
            .unwrap();
        wait_ready(&handle);
        let leased = handle.ready().unwrap();
        let catalogue_dir = root.catalogue_dir(descriptor.catalogue_identity()).unwrap();
        let image_dir = root.image_dir(descriptor.image_identity()).unwrap();
        assert_eq!(root.prune_to(&BTreeSet::new(), 0).unwrap(), 0);
        assert!(catalogue_dir.is_dir());
        assert!(image_dir.is_dir());
        drop(leased);
        let _ = fs::remove_dir_all(root.path());

        let stale_root = temp_root("prune-partial");
        let partial = stale_root.partial_dir(274, ArtifactKind::Images, Digest([7; 32]));
        fs::create_dir_all(&partial).unwrap();
        fs::write(partial.join("stale.bin"), [1u8; 64]).unwrap();
        let stale_lock = entry_lock_path(&partial).unwrap();
        assert!(stale_root.prune_to(&BTreeSet::new(), 0).unwrap() > 0);
        assert!(!partial.exists());
        // Unlinking the lock would let two processes lock different inodes
        // for one key (see same_key_prune_stress_across_processes).
        assert!(stale_lock.is_file());
    }

    #[test]
    fn prune_skips_entry_lock_while_writer_checks_capacity() {
        let root = temp_root("prune-lock-order");
        let entry = root
            .path()
            .join("274")
            .join("catalogues")
            .join("held-entry");
        fs::create_dir_all(&entry).unwrap();
        fs::write(entry.join("payload.bin"), [7u8; 64]).unwrap();
        let entry_lock = entry_lock_path(&entry).unwrap();
        let held = PublishLock::acquire(&entry_lock, None).unwrap();
        let prune_root = root.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        let prune_thread = thread::spawn(move || {
            let capacity_path = prune_root
                .path()
                .join(format!(".capacity{PUBLISH_LOCK_NAME}"));
            let capacity = PublishLock::acquire(&capacity_path, None).unwrap();
            sender.send(()).unwrap();
            let result =
                prune_root.ensure_capacity_locked(MAX_GENERATED_CACHE_BYTES, &BTreeSet::new());
            drop(capacity);
            result
        });

        receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("capacity holder did not start");
        root.ensure_capacity(0, &BTreeSet::new(), None).unwrap();
        let result = prune_thread.join().unwrap();
        assert!(matches!(
            result,
            Err(MapCacheError::Map(MapError::Limit(
                "generated map cache bytes"
            )))
        ));
        assert!(entry.is_dir());

        drop(held);
        assert!(root.prune_to(&BTreeSet::new(), 0).unwrap() > 0);
        assert!(!entry.exists());
    }

    #[test]
    fn catalogue_only_demand_never_invokes_image_stage() {
        let root = temp_root("catalogue-only");
        let producer = FixtureProducer::new(false);
        let manager = MapDemandManager::new(root.clone(), producer.clone());
        let handle = manager
            .request(descriptor(&root), MapDemand::CatalogueOnly)
            .unwrap();
        wait_ready(&handle);
        assert_eq!(producer.rasterized.load(AtomicOrdering::Relaxed), 0);
        assert!(!root.path().join("274/images").exists());
        let _ = fs::remove_dir_all(root.path());
    }

    const STRESS_CHILD_ENV: &str = "274BOT_MAP_CACHE_STRESS_CHILD";

    #[test]
    fn different_key_stress_child() {
        let Ok(index) = std::env::var(STRESS_CHILD_ENV) else {
            return;
        };
        let index: u8 = index.parse().unwrap();
        let root = MapCacheRoot::from_existing_root(
            std::env::var_os("274BOT_MAP_CACHE_STRESS_ROOT").unwrap(),
        );
        let producer = FixtureProducer::new(false);
        let manager = MapDemandManager::new(root.clone(), producer);
        for iteration in 0..20u8 {
            let seed = index * 40 + iteration;
            let descriptor = MapProfileDescriptor::new(
                274,
                Digest([seed.wrapping_add(1); 32]),
                Digest([seed.wrapping_add(41); 32]),
                Digest([seed.wrapping_add(81); 32]),
            )
            .unwrap();
            let handle = manager.request(descriptor, MapDemand::Images).unwrap();
            wait_ready(&handle);
            drop(handle);
        }
    }

    #[test]
    fn different_key_stress_across_processes() {
        let root = temp_root("stress-process");
        let mut children = Vec::new();
        for index in 0..2u8 {
            let child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "map_cache::tests::different_key_stress_child",
                    "--nocapture",
                ])
                .env(STRESS_CHILD_ENV, index.to_string())
                .env("274BOT_MAP_CACHE_STRESS_ROOT", root.path())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            children.push(child);
        }
        let outputs: Vec<_> = children
            .into_iter()
            .map(|child| child.wait_with_output().unwrap())
            .collect();
        if let Some((index, _)) = outputs
            .iter()
            .enumerate()
            .find(|(_, output)| !output.status.success())
        {
            let logs = outputs
                .iter()
                .enumerate()
                .map(|(index, output)| {
                    format!(
                        "child {index}: stdout={} stderr={}",
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            panic!("stress child {index} failed; all child logs:\n{logs}");
        }
    }

    fn wait_terminal(handle: &MapDemandHandle) -> MapJobStatus {
        let start = Instant::now();
        loop {
            let status = handle.status();
            if is_terminal(&status) {
                return status;
            }
            assert!(
                start.elapsed() < Duration::from_secs(30),
                "status: {status:?}"
            );
            thread::sleep(Duration::from_millis(2));
        }
    }

    fn spawn_test_child(test: &str, env: (&str, &Path)) -> std::process::Child {
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test, "--nocapture"])
            .env(env.0, env.1)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }

    fn assert_children_succeeded(children: Vec<std::process::Child>) {
        let outputs: Vec<_> = children
            .into_iter()
            .map(|child| child.wait_with_output().unwrap())
            .collect();
        if outputs.iter().any(|output| !output.status.success()) {
            let logs = outputs
                .iter()
                .enumerate()
                .map(|(index, output)| {
                    format!(
                        "child {index} ({}): stdout={} stderr={}",
                        output.status,
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            panic!("test child failed; all child logs:\n{logs}");
        }
    }

    /// Starts a panel's cold Images bake and parks it inside the image stage,
    /// after its catalogue has been published.
    fn start_held_panel_bake(
        root: &MapCacheRoot,
    ) -> (Arc<FixtureProducer>, MapDemandManager, MapDemandHandle) {
        let producer = FixtureProducer::new(false);
        producer.hold_images.store(true, AtomicOrdering::Release);
        let manager = MapDemandManager::new(root.clone(), producer.clone());
        let handle = manager
            .request(descriptor(root), MapDemand::Images)
            .unwrap();
        let start = Instant::now();
        while !producer.image_started.load(AtomicOrdering::Acquire) {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "status: {:?}",
                handle.status()
            );
            thread::sleep(Duration::from_millis(1));
        }
        (producer, manager, handle)
    }

    fn finish_held_panel_bake(producer: &FixtureProducer, handle: &MapDemandHandle) {
        assert!(
            !matches!(handle.status(), MapJobStatus::Ready),
            "the panel image bake must still be running"
        );
        assert!(!producer.image_exited.load(AtomicOrdering::Acquire));
        producer.hold_images.store(false, AtomicOrdering::Release);
        wait_ready(handle);
        assert!(handle.ready().unwrap().images.is_some());
    }

    #[test]
    fn catalogue_only_request_is_served_during_cold_image_bake_in_other_manager() {
        let root = temp_root("tui-manager-during-panel-bake");
        let (panel_producer, _panel, panel_handle) = start_held_panel_bake(&root);

        let tui_producer = FixtureProducer::new(false);
        let tui = MapDemandManager::new(root.clone(), tui_producer.clone());
        let tui_handle = tui
            .request(descriptor(&root), MapDemand::CatalogueOnly)
            .unwrap();
        wait_ready(&tui_handle);
        assert!(tui_handle.ready().unwrap().images.is_none());
        assert_eq!(tui_producer.runs.load(AtomicOrdering::Relaxed), 0);

        // The panel's image stage protects its catalogue from pruning with a
        // shared lease only, so other readers are never excluded.
        let catalogue_lock = root.lock_path(
            274,
            ArtifactKind::Catalogue,
            descriptor(&root).catalogue_key().unwrap(),
        );
        drop(tui_handle);
        assert!(PublishLock::try_acquire_shared(&catalogue_lock)
            .unwrap()
            .is_some());
        assert!(PublishLock::try_acquire(&catalogue_lock).unwrap().is_none());
        finish_held_panel_bake(&panel_producer, &panel_handle);
    }

    const TUI_CHILD_ENV: &str = "274BOT_MAP_CACHE_TUI_CHILD_ROOT";

    #[test]
    fn catalogue_only_child() {
        let Some(path) = std::env::var_os(TUI_CHILD_ENV) else {
            return;
        };
        let root = MapCacheRoot::from_existing_root(path);
        let producer = FixtureProducer::new(false);
        let manager = MapDemandManager::new(root.clone(), producer.clone());
        let handle = manager
            .request(descriptor(&root), MapDemand::CatalogueOnly)
            .unwrap();
        wait_ready(&handle);
        assert!(handle.ready().unwrap().images.is_none());
        assert_eq!(producer.runs.load(AtomicOrdering::Relaxed), 0);
    }

    #[test]
    fn catalogue_only_request_is_served_during_cold_image_bake_in_other_process() {
        let root = temp_root("tui-process-during-panel-bake");
        let (panel_producer, _panel, panel_handle) = start_held_panel_bake(&root);
        let child = spawn_test_child(
            "map_cache::tests::catalogue_only_child",
            (TUI_CHILD_ENV, root.path()),
        );
        assert_children_succeeded(vec![child]);
        finish_held_panel_bake(&panel_producer, &panel_handle);
    }

    #[test]
    fn worker_ready_map_holds_only_shared_publication_locks() {
        let root = temp_root("worker-shared-leases");
        let descriptor = descriptor(&root);
        let manager = MapDemandManager::new(root.clone(), FixtureProducer::new(false));
        let handle = manager
            .request(descriptor.clone(), MapDemand::Images)
            .unwrap();
        assert!(handle.generation().is_some(), "cold request needs a worker");
        wait_ready(&handle);
        let ready = handle.ready().unwrap();
        let locks = [
            root.lock_path(
                274,
                ArtifactKind::Catalogue,
                descriptor.catalogue_key().unwrap(),
            ),
            root.lock_path(274, ArtifactKind::Images, descriptor.image_key().unwrap()),
        ];
        for lock in &locks {
            // A handle that re-locks shared after holding the exclusive lock
            // keeps both on Windows (LockFileEx); this probe then WouldBlocks.
            assert!(
                PublishLock::try_acquire_shared(lock).unwrap().is_some(),
                "worker ReadyMap still excludes readers of {}",
                lock.display()
            );
            assert!(
                PublishLock::try_acquire(lock).unwrap().is_none(),
                "worker ReadyMap does not protect {}",
                lock.display()
            );
        }
        let other = MapDemandManager::new(root.clone(), FixtureProducer::new(false));
        let other_handle = other.request(descriptor, MapDemand::Images).unwrap();
        assert!(matches!(other_handle.status(), MapJobStatus::Ready));
        drop(other_handle);
        drop(ready);
        drop(handle);
        for lock in &locks {
            assert!(
                PublishLock::try_acquire(lock).unwrap().is_some(),
                "publication lease leaked on {}",
                lock.display()
            );
        }
    }

    #[test]
    fn request_under_same_key_exclusive_lock_queues_instead_of_failing() {
        let root = temp_root("request-exclusive");
        let descriptor = descriptor(&root);
        let manager = MapDemandManager::new(root.clone(), FixtureProducer::new(false));
        let baked = manager
            .request(descriptor.clone(), MapDemand::Images)
            .unwrap();
        wait_ready(&baked);
        drop(baked);
        for (demand, kind, key) in [
            (
                MapDemand::CatalogueOnly,
                ArtifactKind::Catalogue,
                descriptor.catalogue_key().unwrap(),
            ),
            (
                MapDemand::Images,
                ArtifactKind::Images,
                descriptor.image_key().unwrap(),
            ),
        ] {
            // Another process publishing or pruning this key holds it
            // exclusively; the ready directory is still on disk.
            let held = PublishLock::try_acquire(&root.lock_path(274, kind, key))
                .unwrap()
                .expect("no lease remains after the handles dropped");
            let producer = FixtureProducer::new(false);
            let other = MapDemandManager::new(root.clone(), producer.clone());
            let handle = other
                .request(descriptor.clone(), demand)
                .expect("a same-key exclusive lock is not a request error");
            thread::sleep(Duration::from_millis(50));
            assert!(!is_terminal(&handle.status()), "{:?}", handle.status());
            drop(held);
            wait_ready(&handle);
            assert_eq!(
                handle.ready().unwrap().images.is_some(),
                demand == MapDemand::Images
            );
            assert_eq!(producer.runs.load(AtomicOrdering::Relaxed), 0);
        }
    }

    const SAME_KEY_CHILD_ENV: &str = "274BOT_MAP_CACHE_SAME_KEY_CHILD_ROOT";
    const SAME_KEY_PROCESSES: usize = 3;
    const SAME_KEY_THREADS: usize = 4;
    const SAME_KEY_ITERATIONS: usize = 40;

    #[test]
    fn same_key_prune_stress_child() {
        let Some(path) = std::env::var_os(SAME_KEY_CHILD_ENV) else {
            return;
        };
        let descriptors = [
            MapProfileDescriptor::new(274, Digest([11; 32]), Digest([12; 32]), Digest([13; 32]))
                .unwrap(),
            MapProfileDescriptor::new(274, Digest([21; 32]), Digest([22; 32]), Digest([23; 32]))
                .unwrap(),
        ];
        let workers: Vec<_> = (0..SAME_KEY_THREADS)
            .map(|thread_index| {
                let root = MapCacheRoot::from_existing_root(&path);
                let descriptors = descriptors.clone();
                thread::spawn(move || {
                    let manager = MapDemandManager::new(root.clone(), FixtureProducer::new(false));
                    let capacity = root.path().join(format!(".capacity{PUBLISH_LOCK_NAME}"));
                    let mut failures = Vec::new();
                    for iteration in 0..SAME_KEY_ITERATIONS {
                        let descriptor = descriptors[(thread_index + iteration) % 2].clone();
                        match manager.request(descriptor, MapDemand::Images) {
                            Err(error) => failures.push(format!("request: {error}")),
                            Ok(handle) => match wait_terminal(&handle) {
                                MapJobStatus::Ready => {
                                    if let Err(error) = handle.ready() {
                                        failures.push(format!("ready: {error}"));
                                    }
                                }
                                status => failures.push(format!("bake: {status:?}")),
                            },
                        }
                        // Forced eviction of every unleased entry, as a
                        // capacity-pressured writer in another process does.
                        let capacity = PublishLock::acquire(&capacity, None).unwrap();
                        if let Err(error) = root.prune_to(&BTreeSet::new(), 0) {
                            failures.push(format!("prune: {error}"));
                        }
                        drop(capacity);
                    }
                    failures
                })
            })
            .collect();
        let failures: Vec<String> = workers
            .into_iter()
            .flat_map(|worker| worker.join().unwrap())
            .collect();
        assert!(
            failures.is_empty(),
            "{} failures: {failures:?}",
            failures.len()
        );
    }

    #[test]
    fn same_key_prune_stress_across_processes() {
        let root = temp_root("same-key-prune-stress");
        fs::create_dir_all(root.path()).unwrap();
        let children = (0..SAME_KEY_PROCESSES)
            .map(|_| {
                spawn_test_child(
                    "map_cache::tests::same_key_prune_stress_child",
                    (SAME_KEY_CHILD_ENV, root.path()),
                )
            })
            .collect();
        assert_children_succeeded(children);
    }

    #[test]
    fn corrupt_ready_path_is_rejected_without_overwrite() {
        let root = temp_root("corrupt-ready");
        let descriptor = descriptor(&root);
        let catalogue = root.catalogue_dir(descriptor.catalogue_identity()).unwrap();
        fs::create_dir_all(catalogue.parent().unwrap()).unwrap();
        fs::write(&catalogue, b"not-a-directory").unwrap();
        let producer = FixtureProducer::new(false);
        let manager = MapDemandManager::new(root.clone(), producer);
        match manager.request(descriptor, MapDemand::CatalogueOnly) {
            Err(MapCacheError::Map(MapError::Path)) => {}
            other => {
                let _ = other;
                panic!("corrupt ready path was not rejected");
            }
        }
        assert_eq!(fs::read(catalogue).unwrap(), b"not-a-directory");
        let _ = fs::remove_dir_all(root.path());
    }

    #[test]
    fn publish_lock_serializes_live_owner() {
        let root = temp_root("lock");
        let key = Digest([9; 32]);
        let lock_path = root
            .path()
            .join("274/catalogues")
            .join(format!(".{key}{PUBLISH_LOCK_NAME}"));
        fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
        let lock = PublishLock::acquire(&lock_path, None).unwrap();
        let acquired = Arc::new(AtomicBool::new(false));
        let contender = thread::spawn({
            let lock_path = lock_path.clone();
            let acquired = Arc::clone(&acquired);
            move || {
                let guard = PublishLock::acquire(&lock_path, None).unwrap();
                acquired.store(true, AtomicOrdering::Release);
                drop(guard);
            }
        });
        thread::sleep(Duration::from_millis(50));
        assert!(!acquired.load(AtomicOrdering::Acquire));
        drop(lock);
        contender.join().unwrap();
        assert!(acquired.load(AtomicOrdering::Acquire));
        let reacquired = PublishLock::acquire(&lock_path, None).unwrap();
        drop(reacquired);
        let _ = fs::remove_dir_all(root.path());
    }
    const LOCK_CHILD_ENV: &str = "274BOT_MAP_CACHE_LOCK_CHILD";

    #[test]
    fn publish_lock_child() {
        let Ok(path) = std::env::var(LOCK_CHILD_ENV) else {
            return;
        };
        let lock_path = PathBuf::from(path);
        let _lock = PublishLock::acquire(&lock_path, None).unwrap();
        println!("LOCK_READY");
        let mut stdout = std::io::stdout();
        std::io::Write::flush(&mut stdout).unwrap();
        let mut stdin = std::io::stdin();
        let mut input = Vec::new();
        let _ = stdin.read_to_end(&mut input);
    }

    #[test]
    fn publish_lock_excludes_other_process() {
        let root = temp_root("lock-process");
        let lock_path = root.path().join("process.publish.lock");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "map_cache::tests::publish_lock_child",
                "--nocapture",
            ])
            .env(LOCK_CHILD_ENV, &lock_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut reader = BufReader::new(stdout);
        let mut output = String::new();
        let start = Instant::now();
        while !output.contains("LOCK_READY") {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap() == 0 {
                break;
            }
            output.push_str(&line);
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "child output: {output}"
            );
        }
        assert!(output.contains("LOCK_READY"), "child output: {output}");
        assert!(PublishLock::try_acquire(&lock_path).unwrap().is_none());
        drop(child.stdin.take());
        assert!(child.wait().unwrap().success());
        let lock = PublishLock::try_acquire(&lock_path).unwrap();
        assert!(lock.is_some(), "lock was not released by child");
        drop(lock);
    }
}
