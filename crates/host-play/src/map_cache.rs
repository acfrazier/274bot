//! Persistent, demand-driven WalkTo map resources.
//!
//! Profile preparation creates an immutable [`MapProfileDescriptor`] only.
//! The first explicit demand submits bounded cooperative work to the shared
//! WorkAdmission service. The host owns demand, persistence and publication;
//! navigation owns map decoding/raster policy. Workers write only
//! identity-keyed `.partial` directories and publish a complete artifact with
//! one cross-process lock; readers never inspect a partial directory.

use api::identity::{Cancellation, PublishSequence, Scope, Settlement, WorkIdentity};
use api::process_condition::ProcessConditions;
use api::work::{
    Checkpoint as WorkCheckpoint, CooperativeJob, InputRevision, JobError, JobSettlement,
    MapBakePhase, RetainedState, StepContext, StepOutcome, SubmissionRefusal, WorkAdmission,
    WorkCharge, WorkFamily, WorkFamilySpec, WorkTicket,
};
use client::map_cache::MAP_PLANES;
use client::unpack::PreparedRuntimeCache;
use nav::map::cache::{
    ArtifactIdentity, BakeStage, Checkpoint, CompletedUnit, ReadyCatalogue as NavReadyCatalogue,
    ReadyImages as NavReadyImages, UnitKey, CHECKPOINT_SCHEMA,
};
use nav::map::formats::{
    CatalogueManifest, ClientPois, ImageManifest, PayloadReceipt, ShippedImages, TileReceipt,
    MAX_IMAGE_BYTES, MAX_IMAGE_TILES, MAX_JSON_BYTES, MAX_PNG_BYTES,
};
use nav::map::identity::{CatalogueIdentity, Digest, ImageIdentity, CATALOGUE_SCHEMA};
use nav::map::raster::CHECKPOINT_BATCH_TILES;
use nav::map::MapError;
use parking_lot::Mutex;
use script::work_admission::WorkService;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering as AtomicOrdering};
use std::sync::{Arc, Weak};
#[cfg(test)]
use std::thread;
use std::time::{Duration, SystemTime};

pub const PUBLISH_LOCK_NAME: &str = ".publish.lock";
pub const MAX_GENERATED_CACHE_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_IMAGE_KEY_BYTES: u64 = 128 * 1024 * 1024;
const MAX_CATALOGUE_KEY_BYTES: u64 = MAX_JSON_BYTES as u64;
/// Twice the largest complete artifact from revisions 274/289: 2 * 1,745.
/// Evidence: TRACK-T5/T5W/map-bake/cache-shape-measurement-r1/measurements.json.
const MAX_ARTIFACT_ROOT_ENTRIES: usize = 3_490;
/// The unchanged byte cap divided by the smallest measured complete artifact:
/// floor(268,435,456 / 260,457) = 1,030 roots.
const MAX_CACHE_ARTIFACT_ROOTS: usize = 1_030;
/// Bound streamed entries, not retained names: 3,490 * 1,030 = 3,594,700.
const MAX_CACHE_SCAN_ENTRIES: u64 = 3_594_700;
const MAX_CENSUS_PATH_BYTES: usize = 4_096;
const MAX_CENSUS_NAME_BYTES: usize = 1_024;
const PRUNE_MARKER_NAME: &str = ".prune-in-progress";
const CAPACITY_GENERATION_BYTES: u64 = std::mem::size_of::<u64>() as u64;
const CAPACITY_LOCK_FILE_NAME: &str = ".capacity.publish.lock";
const MAP_BAKE_PATH_SUFFIX_RESERVE_BYTES: usize = 128;
const MAP_BAKE_PATH_OWNER_COUNT: usize = 6;
const MAP_BAKE_PATH_NODE_BYTES: usize = 128;
const MAP_BAKE_REQUIRED_PATH_NODE_COUNT: usize = 4;
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(1);
/// Coalesce unknown-denominator progress to one report per 1,024 cursor units.
const MAP_PROGRESS_UNKNOWN_STEP: u32 = 1_024;

#[cfg(test)]
static MAP_BAKE_TEST_DISPATCHES: AtomicU64 = AtomicU64::new(0);
#[cfg(test)]
static MAP_BAKE_TEST_UNITS: AtomicU64 = AtomicU64::new(0);
#[cfg(test)]
static MAP_BAKE_TEST_DISPATCH_NANOS: AtomicU64 = AtomicU64::new(0);
#[cfg(test)]
static MAP_BAKE_TEST_MAX_DISPATCH_NANOS: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
#[derive(Debug, Clone, Copy)]
struct MapBakeTestStepSnapshot {
    dispatches: u64,
    units: u64,
    dispatch_nanos: u64,
    max_dispatch_nanos: u64,
}

#[cfg(test)]
fn map_bake_test_step_snapshot() -> MapBakeTestStepSnapshot {
    MapBakeTestStepSnapshot {
        dispatches: MAP_BAKE_TEST_DISPATCHES.load(AtomicOrdering::Relaxed),
        units: MAP_BAKE_TEST_UNITS.load(AtomicOrdering::Relaxed),
        dispatch_nanos: MAP_BAKE_TEST_DISPATCH_NANOS.load(AtomicOrdering::Relaxed),
        max_dispatch_nanos: MAP_BAKE_TEST_MAX_DISPATCH_NANOS.load(AtomicOrdering::Relaxed),
    }
}

#[cfg(test)]
struct MapBakeTestStepTimer(std::time::Instant);

#[cfg(test)]
impl MapBakeTestStepTimer {
    fn start() -> Self {
        Self(std::time::Instant::now())
    }
}

#[cfg(test)]
impl Drop for MapBakeTestStepTimer {
    fn drop(&mut self) {
        let elapsed_nanos = self.0.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
        MAP_BAKE_TEST_DISPATCHES.fetch_add(1, AtomicOrdering::Relaxed);
        MAP_BAKE_TEST_DISPATCH_NANOS.fetch_add(elapsed_nanos, AtomicOrdering::Relaxed);
        MAP_BAKE_TEST_MAX_DISPATCH_NANOS.fetch_max(elapsed_nanos, AtomicOrdering::Relaxed);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ArtifactRootId {
    revision: u16,
    kind: ArtifactKind,
    key: Digest,
    partial: bool,
}

#[derive(Debug, Clone)]
struct ArtifactRootSummary {
    id: ArtifactRootId,
    bytes: u64,
    entries: usize,
    lru: SystemTime,
    identity: Option<ArtifactIdentity>,
    fully_censused: bool,
    safe_to_prune: bool,
    /// Publication custody observed during this census. Pruning rechecks it.
    publication_busy: bool,
    marked_for_prune: bool,
    root_exists: bool,
}

#[derive(Debug, Clone, Copy)]
struct KindDirectoryEntry {
    id: ArtifactRootId,
    root_directory: bool,
    prune_marker: bool,
    lru: SystemTime,
    identity: Option<ArtifactIdentity>,
}

#[derive(Debug)]
struct CacheCensusResult {
    generation: u64,
    bytes: u64,
    entries: u64,
    product_entries: u64,
    roots: Vec<ArtifactRootSummary>,
    overrun: Option<CacheCensusOverrun>,
    foreign_tree: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CacheCensusOverrun {
    RootSummaryCapacity,
    ArtifactRootEntries,
}

#[derive(Debug)]
struct CensusDirectory {
    parent: Option<usize>,
    name: OsString,
}

#[derive(Debug)]
struct ArtifactTreeCursor {
    directories: Vec<CensusDirectory>,
    pending: Vec<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CacheCensusPhase {
    CacheRoot,
    Revision(usize),
    Kind(usize),
    Artifact(usize),
    Complete,
}

struct CacheCensusCursor {
    root: MapCacheRoot,
    expected_generation: Option<u64>,
    phase: CacheCensusPhase,
    revisions: Vec<u16>,
    kind_directories: Vec<(u16, ArtifactKind)>,
    kind_entries: Vec<KindDirectoryEntry>,
    roots: Vec<ArtifactRootSummary>,
    tree: Option<ArtifactTreeCursor>,
    bytes: u64,
    entries: u64,
    product_entries: u64,
    overrun: Option<CacheCensusOverrun>,
    foreign_tree: bool,
}

const PRUNE_MARKER_SCHEMA: u16 = 1;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PruneMarker {
    schema: u16,
    identity: ArtifactIdentity,
    partial: bool,
}

/// Error returned by cache layout, publication and worker ownership code.
#[derive(Debug)]
pub enum MapCacheError {
    Io(io::Error),
    Map(MapError),
    Message(String),
    Cancelled,
    NoPreparedCache,
    Busy,
    /// A [`MapDemand::ReadyImages`] demand found no published terrain; it
    /// never bakes, so the demand settles catalogue-only.
    TerrainNotBaked,
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
            Self::Busy => f.write_str("map cache is temporarily busy"),
            Self::TerrainNotBaked => f.write_str("map terrain is not baked"),
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

    fn try_acquire_capacity(&self) -> Result<Option<CapacityLock>, MapCacheError> {
        match fs::symlink_metadata(&self.0) {
            Ok(metadata) if metadata.file_type().is_dir() => {}
            Ok(_) => return Err(MapError::Invalid("map cache root is not a real directory").into()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        CapacityLock::try_acquire(&self.0.join(CAPACITY_LOCK_FILE_NAME))
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

    fn artifact_root_path(&self, id: ArtifactRootId) -> PathBuf {
        if id.partial {
            self.partial_dir(id.revision, id.kind, id.key)
        } else {
            self.kind_dir(id.revision, id.kind).join(id.key.to_string())
        }
    }

    fn prune_marker_path(&self, id: ArtifactRootId) -> PathBuf {
        self.kind_dir(id.revision, id.kind)
            .join(format!(".{}{PRUNE_MARKER_NAME}", id.key))
    }

    fn lock_path(&self, revision: u16, kind: ArtifactKind, key: Digest) -> PathBuf {
        self.kind_dir(revision, kind)
            .join(format!(".{key}{PUBLISH_LOCK_NAME}"))
    }

    fn ensure_kind_dir(&self, revision: u16, kind: ArtifactKind) -> Result<bool, MapCacheError> {
        let revision_dir = self.revision_dir(revision);
        let kind_dir = self.kind_dir(revision, kind);
        let mut changed = false;
        for (path, parent) in [(&revision_dir, &self.0), (&kind_dir, &revision_dir)] {
            match fs::symlink_metadata(path) {
                Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                    return Err(MapCacheError::Map(MapError::Path));
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    match fs::create_dir(path) {
                        Ok(()) => {
                            sync_directory(parent)?;
                            changed = true;
                        }
                        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                            if !real_directory(path)? {
                                return Err(MapCacheError::Map(MapError::Path));
                            }
                        }
                        Err(error) => return Err(error.into()),
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(changed)
    }

    fn ensure_key_lock_file(
        &self,
        revision: u16,
        kind: ArtifactKind,
        key: Digest,
    ) -> Result<bool, MapCacheError> {
        let path = self.lock_path(revision, kind, key);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                Err(MapCacheError::Map(MapError::Path))
            }
            Ok(_) => Ok(false),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                match OpenOptions::new().write(true).create_new(true).open(&path) {
                    // The empty key file coordinates live processes. It has
                    // no artifact data to preserve across a host restart.
                    Ok(_file) => Ok(true),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                        let metadata = fs::symlink_metadata(&path)?;
                        if metadata.file_type().is_symlink() || !metadata.is_file() {
                            return Err(MapCacheError::Map(MapError::Path));
                        }
                        Ok(false)
                    }
                    Err(error) => Err(error.into()),
                }
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Open only after the admitted job's bounded census accepts this cache
    /// and requested artifact root.
    fn open_catalogue(
        &self,
        identity: CatalogueIdentity,
        capacity: &mut CapacityLock,
    ) -> Result<Option<Arc<ReadyCatalogue>>, MapCacheError> {
        let path = self.catalogue_dir(identity)?;
        let key = identity.key()?.0;
        let lock_path = self.lock_path(identity.revision, ArtifactKind::Catalogue, key);
        let marker_path = self.prune_marker_path(ArtifactRootId {
            revision: identity.revision,
            kind: ArtifactKind::Catalogue,
            key,
            partial: false,
        });
        let Some(lock) = ready_lease(
            &lock_path,
            &path,
            &marker_path,
            ArtifactRootId {
                revision: identity.revision,
                kind: ArtifactKind::Catalogue,
                key,
                partial: false,
            },
            ArtifactIdentity::Catalogue(identity),
        )?
        else {
            return Ok(None);
        };
        clear_ready_checkpoint(&path, capacity)?;
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

    /// Open only after the admitted job's bounded census accepts this cache
    /// and requested artifact root; tile PNGs remain unopened until requested.
    fn open_images(
        &self,
        identity: ImageIdentity,
        capacity: &mut CapacityLock,
    ) -> Result<Option<Arc<ReadyImages>>, MapCacheError> {
        let path = self.image_dir(identity)?;
        let key = identity.key()?.0;
        let lock_path = self.lock_path(identity.revision, ArtifactKind::Images, key);
        let marker_path = self.prune_marker_path(ArtifactRootId {
            revision: identity.revision,
            kind: ArtifactKind::Images,
            key,
            partial: false,
        });
        let Some(lock) = ready_lease(
            &lock_path,
            &path,
            &marker_path,
            ArtifactRootId {
                revision: identity.revision,
                kind: ArtifactKind::Images,
                key,
                partial: false,
            },
            ArtifactIdentity::Image(identity),
        )?
        else {
            return Ok(None);
        };
        clear_ready_checkpoint(&path, capacity)?;
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
}

fn compare_revision_path(a: u16, b: u16) -> Ordering {
    let digits = |value: u16| {
        if value >= 10_000 {
            5
        } else if value >= 1_000 {
            4
        } else if value >= 100 {
            3
        } else if value >= 10 {
            2
        } else {
            1
        }
    };
    digits(a).cmp(&digits(b)).then_with(|| a.cmp(&b))
}

fn compare_root_id(a: ArtifactRootId, b: ArtifactRootId) -> Ordering {
    compare_revision_path(a.revision, b.revision)
        .then_with(|| a.kind.directory_name().cmp(b.kind.directory_name()))
        .then_with(|| b.partial.cmp(&a.partial))
        .then_with(|| a.key.cmp(&b.key))
}

fn parse_revision_name(name: &std::ffi::OsStr) -> Option<u16> {
    let text = name.to_str()?;
    let revision = text.parse::<u16>().ok()?;
    (revision.to_string() == text).then_some(revision)
}

fn parse_digest_name(text: &str) -> Option<Digest> {
    (text.len() == 64)
        .then(|| Digest::from_hex(text).ok())
        .flatten()
}

fn parse_artifact_root_name(
    name: &std::ffi::OsStr,
    revision: u16,
    kind: ArtifactKind,
) -> Option<ArtifactRootId> {
    let name = name.to_str()?;
    let (digest, partial) = match name
        .strip_prefix('.')
        .and_then(|name| name.strip_suffix(".partial").map(|digest| (digest, true)))
    {
        Some((digest, partial)) => (digest, partial),
        None if !name.starts_with('.') => (name, false),
        None => return None,
    };
    Some(ArtifactRootId {
        revision,
        kind,
        key: parse_digest_name(digest)?,
        partial,
    })
}

fn parse_keyed_control_name(name: &std::ffi::OsStr, suffix: &str) -> Option<Digest> {
    let name = name.to_str()?;
    let digest = name.strip_prefix('.')?.strip_suffix(suffix)?;
    parse_digest_name(digest)
}

fn identity_matches_root(identity: ArtifactIdentity, root: ArtifactRootId) -> bool {
    match identity {
        ArtifactIdentity::Catalogue(identity) => {
            root.kind == ArtifactKind::Catalogue
                && identity.revision == root.revision
                && identity.key().is_ok_and(|key| key.0 == root.key)
        }
        ArtifactIdentity::Image(identity) => {
            root.kind == ArtifactKind::Images
                && identity.revision == root.revision
                && identity.key().is_ok_and(|key| key.0 == root.key)
        }
    }
}

fn read_regular_bounded(path: &Path, limit: usize) -> Result<Option<Vec<u8>>, MapCacheError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(MapCacheError::Map(MapError::Path));
    }
    if metadata.len() > limit as u64 {
        return Ok(None);
    }
    read_bounded(path, limit).map(Some)
}

fn read_prune_marker(
    path: &Path,
    revision: u16,
    kind: ArtifactKind,
    key: Digest,
) -> Result<Option<(ArtifactRootId, ArtifactIdentity)>, MapCacheError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > 256 {
        return Err(MapCacheError::Map(MapError::Path));
    }
    let bytes = read_bounded(path, 256)?;
    let marker: PruneMarker =
        serde_json::from_slice(&bytes).map_err(|_| MapCacheError::Map(MapError::Path))?;
    let id = ArtifactRootId {
        revision,
        kind,
        key,
        partial: marker.partial,
    };
    if marker.schema != PRUNE_MARKER_SCHEMA || !identity_matches_root(marker.identity, id) {
        return Err(MapCacheError::Map(MapError::Path));
    }
    Ok(Some((id, marker.identity)))
}

fn read_root_identity(
    path: &Path,
    id: ArtifactRootId,
) -> Result<Option<ArtifactIdentity>, MapCacheError> {
    let Some(bytes) = read_regular_bounded(&path.join("manifest.json"), MAX_JSON_BYTES)? else {
        if id.partial {
            let Some(bytes) = read_regular_bounded(&path.join("checkpoint.json"), MAX_JSON_BYTES)?
            else {
                return Ok(None);
            };
            let Ok(checkpoint) = serde_json::from_slice::<Checkpoint>(&bytes) else {
                return Ok(None);
            };
            if !identity_matches_root(checkpoint.identity, id)
                || Checkpoint::decode(&bytes, checkpoint.identity).is_err()
            {
                return Ok(None);
            }
            return Ok(Some(checkpoint.identity));
        }
        return Ok(None);
    };
    match id.kind {
        ArtifactKind::Catalogue => {
            #[derive(Deserialize)]
            struct Header {
                identity: CatalogueIdentity,
            }
            let Ok(header) = serde_json::from_slice::<Header>(&bytes) else {
                return Ok(None);
            };
            if !identity_matches_root(ArtifactIdentity::Catalogue(header.identity), id)
                || CatalogueManifest::decode(&bytes, header.identity).is_err()
            {
                return Ok(None);
            }
            Ok(Some(ArtifactIdentity::Catalogue(header.identity)))
        }
        ArtifactKind::Images => {
            #[derive(Deserialize)]
            struct Header {
                identity: ImageIdentity,
            }
            let Ok(header) = serde_json::from_slice::<Header>(&bytes) else {
                return Ok(None);
            };
            if !identity_matches_root(ArtifactIdentity::Image(header.identity), id)
                || ImageManifest::decode(&bytes, header.identity).is_err()
            {
                return Ok(None);
            }
            Ok(Some(ArtifactIdentity::Image(header.identity)))
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum CensusEntryKind {
    Directory,
    File,
    Unsafe,
}

impl CacheCensusCursor {
    fn new(root: MapCacheRoot) -> Self {
        Self {
            root,
            expected_generation: None,
            phase: CacheCensusPhase::CacheRoot,
            revisions: Vec::with_capacity(MAX_CACHE_ARTIFACT_ROOTS),
            kind_directories: Vec::with_capacity(2 * MAX_CACHE_ARTIFACT_ROOTS),
            kind_entries: Vec::with_capacity(MAX_ARTIFACT_ROOT_ENTRIES),
            roots: Vec::with_capacity(MAX_CACHE_ARTIFACT_ROOTS),
            tree: None,
            bytes: 0,
            entries: 0,
            product_entries: 0,
            overrun: None,
            foreign_tree: false,
        }
    }

    fn reset(&mut self) {
        self.phase = CacheCensusPhase::CacheRoot;
        self.revisions.clear();
        self.kind_directories.clear();
        self.kind_entries.clear();
        self.roots.clear();
        self.tree = None;
        self.bytes = 0;
        self.entries = 0;
        self.product_entries = 0;
        self.overrun = None;
        self.foreign_tree = false;
    }

    fn advance(&mut self) -> Result<Option<CacheCensusResult>, MapCacheError> {
        let Some(capacity) = self.root.try_acquire_capacity()? else {
            return Ok(None);
        };
        let generation = capacity.generation();
        if self
            .expected_generation
            .is_some_and(|current| current != generation)
        {
            self.reset();
        }
        self.expected_generation = Some(generation);
        self.advance_locked()?;
        if self.phase != CacheCensusPhase::Complete {
            return Ok(None);
        }
        Ok(Some(CacheCensusResult {
            generation,
            bytes: self.bytes,
            entries: self.entries,
            product_entries: self.product_entries,
            roots: std::mem::take(&mut self.roots),
            overrun: self.overrun,
            foreign_tree: self.foreign_tree,
        }))
    }

    fn advance_locked(&mut self) -> Result<(), MapCacheError> {
        loop {
            match self.phase {
                CacheCensusPhase::CacheRoot => {
                    self.scan_cache_root()?;
                    self.phase = if self.foreign_tree {
                        CacheCensusPhase::Complete
                    } else {
                        CacheCensusPhase::Revision(0)
                    };
                    return Ok(());
                }
                CacheCensusPhase::Revision(index) if index < self.revisions.len() => {
                    let revision = self.revisions[index];
                    self.scan_revision(revision)?;
                    self.phase = if self.foreign_tree {
                        CacheCensusPhase::Complete
                    } else {
                        CacheCensusPhase::Revision(index + 1)
                    };
                    return Ok(());
                }
                CacheCensusPhase::Revision(_) => {
                    self.kind_directories.sort_by(|a, b| {
                        compare_revision_path(a.0, b.0)
                            .then_with(|| a.1.directory_name().cmp(b.1.directory_name()))
                    });
                    self.phase = CacheCensusPhase::Kind(0);
                }
                CacheCensusPhase::Kind(index)
                    if index < self.kind_directories.len() && self.overrun.is_none() =>
                {
                    let (revision, kind) = self.kind_directories[index];
                    self.scan_kind_directory(revision, kind)?;
                    self.phase = if self.foreign_tree || self.overrun.is_some() {
                        CacheCensusPhase::Artifact(0)
                    } else {
                        CacheCensusPhase::Kind(index + 1)
                    };
                    return Ok(());
                }
                CacheCensusPhase::Kind(_) => {
                    self.phase = CacheCensusPhase::Artifact(0);
                }
                CacheCensusPhase::Artifact(index) if index < self.roots.len() => {
                    self.scan_artifact_root(index)?;
                    return Ok(());
                }
                CacheCensusPhase::Artifact(_) => {
                    self.phase = CacheCensusPhase::Complete;
                }
                CacheCensusPhase::Complete => return Ok(()),
            }
        }
    }

    fn observe_global(
        &mut self,
        metadata: &fs::Metadata,
    ) -> Result<CensusEntryKind, MapCacheError> {
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            self.foreign_tree = true;
            return Ok(CensusEntryKind::Unsafe);
        }
        if metadata.is_dir() {
            self.entries =
                self.entries
                    .checked_add(1)
                    .ok_or(MapCacheError::Map(MapError::Limit(
                        "map-cache census entries",
                    )))?;
            Ok(CensusEntryKind::Directory)
        } else if metadata.is_file() {
            self.entries =
                self.entries
                    .checked_add(1)
                    .ok_or(MapCacheError::Map(MapError::Limit(
                        "map-cache census entries",
                    )))?;
            self.bytes = self
                .bytes
                .checked_add(metadata.len())
                .ok_or(MapCacheError::Map(MapError::Limit(
                    "map-cache census bytes",
                )))?;
            Ok(CensusEntryKind::File)
        } else {
            self.foreign_tree = true;
            Ok(CensusEntryKind::Unsafe)
        }
    }

    fn scan_cache_root(&mut self) -> Result<(), MapCacheError> {
        let metadata = fs::symlink_metadata(self.root.path())?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(MapCacheError::Map(MapError::Path));
        }
        self.entries = 1;
        let mut scanned = 0usize;
        for entry in fs::read_dir(self.root.path())? {
            scanned += 1;
            if scanned > MAX_ARTIFACT_ROOT_ENTRIES {
                self.overrun = Some(CacheCensusOverrun::RootSummaryCapacity);
                break;
            }
            let entry = entry?;
            let name = entry.file_name();
            let metadata = fs::symlink_metadata(entry.path())?;
            match self.observe_global(&metadata)? {
                CensusEntryKind::Directory => {
                    if let Some(revision) = parse_revision_name(&name) {
                        if !matches!(revision, 274 | 289)
                            || self.revisions.len() == MAX_CACHE_ARTIFACT_ROOTS
                        {
                            self.foreign_tree = true;
                            break;
                        }
                        self.revisions.push(revision);
                    } else {
                        self.foreign_tree = true;
                        break;
                    }
                }
                CensusEntryKind::File => {}
                CensusEntryKind::Unsafe => break,
            }
        }
        self.revisions.sort_by(|a, b| compare_revision_path(*a, *b));
        Ok(())
    }

    fn scan_revision(&mut self, revision: u16) -> Result<(), MapCacheError> {
        let mut scanned = 0usize;
        for entry in fs::read_dir(self.root.revision_dir(revision))? {
            scanned += 1;
            if scanned > MAX_ARTIFACT_ROOT_ENTRIES {
                self.overrun = Some(CacheCensusOverrun::RootSummaryCapacity);
                break;
            }
            let entry = entry?;
            let name = entry.file_name();
            let metadata = fs::symlink_metadata(entry.path())?;
            match self.observe_global(&metadata)? {
                CensusEntryKind::Directory => match name.to_str() {
                    Some("catalogues") => {
                        self.kind_directories
                            .push((revision, ArtifactKind::Catalogue));
                    }
                    Some("images") => {
                        self.kind_directories.push((revision, ArtifactKind::Images));
                    }
                    _ => {
                        self.foreign_tree = true;
                        break;
                    }
                },
                CensusEntryKind::File => {}
                CensusEntryKind::Unsafe => break,
            }
        }
        Ok(())
    }

    fn scan_kind_directory(
        &mut self,
        revision: u16,
        kind: ArtifactKind,
    ) -> Result<(), MapCacheError> {
        self.kind_entries.clear();
        let mut scanned = 0usize;
        for entry in fs::read_dir(self.root.kind_dir(revision, kind))? {
            scanned += 1;
            if scanned > MAX_ARTIFACT_ROOT_ENTRIES {
                self.overrun = Some(CacheCensusOverrun::RootSummaryCapacity);
                break;
            }
            let entry = entry?;
            let name = entry.file_name();
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            match self.observe_global(&metadata)? {
                CensusEntryKind::Directory => {
                    if let Some(id) = parse_artifact_root_name(&name, revision, kind) {
                        if self.kind_entries.len() == MAX_ARTIFACT_ROOT_ENTRIES {
                            self.overrun = Some(CacheCensusOverrun::RootSummaryCapacity);
                            break;
                        }
                        self.kind_entries.push(KindDirectoryEntry {
                            id,
                            root_directory: true,
                            prune_marker: false,
                            lru: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                            identity: None,
                        });
                    } else {
                        self.foreign_tree = true;
                        break;
                    }
                }
                CensusEntryKind::File => {
                    if let Some(key) = parse_keyed_control_name(&name, PRUNE_MARKER_NAME) {
                        match read_prune_marker(&path, revision, kind, key)? {
                            Some((id, identity)) => {
                                if self.kind_entries.len() == MAX_ARTIFACT_ROOT_ENTRIES {
                                    self.overrun = Some(CacheCensusOverrun::RootSummaryCapacity);
                                    break;
                                }
                                self.kind_entries.push(KindDirectoryEntry {
                                    id,
                                    root_directory: false,
                                    prune_marker: true,
                                    lru: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                                    identity: Some(identity),
                                });
                            }
                            None => {
                                self.foreign_tree = true;
                                break;
                            }
                        }
                    } else if parse_keyed_control_name(&name, PUBLISH_LOCK_NAME).is_none() {
                        // Foreign regular files are charged but never deleted.
                    }
                }
                CensusEntryKind::Unsafe => break,
            }
        }
        self.kind_entries
            .sort_by(|a, b| compare_root_id(a.id, b.id));
        let mut index = 0;
        while index < self.kind_entries.len() {
            let id = self.kind_entries[index].id;
            let mut end = index + 1;
            while end < self.kind_entries.len() && self.kind_entries[end].id == id {
                end += 1;
            }
            if self.roots.len() == MAX_CACHE_ARTIFACT_ROOTS {
                self.overrun = Some(CacheCensusOverrun::RootSummaryCapacity);
                break;
            }
            let group = &self.kind_entries[index..end];
            let root_entry = group.iter().find(|entry| entry.root_directory);
            let marker_entry = group.iter().find(|entry| entry.prune_marker);
            let root_exists = root_entry.is_some();
            let lru = root_entry
                .or(marker_entry)
                .map_or(SystemTime::UNIX_EPOCH, |entry| entry.lru);
            self.roots.push(ArtifactRootSummary {
                id,
                bytes: 0,
                entries: usize::from(root_exists),
                lru,
                identity: marker_entry.and_then(|entry| entry.identity),
                fully_censused: false,
                safe_to_prune: marker_entry.is_some(),
                publication_busy: false,
                marked_for_prune: marker_entry.is_some(),
                root_exists,
            });
            if root_exists {
                self.product_entries =
                    self.product_entries
                        .checked_add(1)
                        .ok_or(MapCacheError::Map(MapError::Limit(
                            "map-cache census entries",
                        )))?;
            }
            index = end;
        }
        Ok(())
    }

    fn scan_artifact_root(&mut self, root_index: usize) -> Result<(), MapCacheError> {
        if self.tree.is_none() {
            let (id, root_exists, marked_for_prune) = {
                let summary = &self.roots[root_index];
                (summary.id, summary.root_exists, summary.marked_for_prune)
            };
            if !root_exists {
                self.roots[root_index].fully_censused = true;
                self.phase = CacheCensusPhase::Artifact(root_index + 1);
                return Ok(());
            }
            let path = self.root.artifact_root_path(id);
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                self.roots[root_index].safe_to_prune = false;
                self.foreign_tree = true;
                self.phase = CacheCensusPhase::Complete;
                return Ok(());
            }
            if !marked_for_prune {
                match read_root_identity(&path, id) {
                    Ok(identity) => {
                        let summary = &mut self.roots[root_index];
                        summary.identity = identity;
                        summary.safe_to_prune = summary.identity.is_some();
                    }
                    Err(MapCacheError::Map(MapError::Path)) => {
                        self.roots[root_index].safe_to_prune = false;
                    }
                    Err(error) => return Err(error),
                }
            }
            if self.roots[root_index].safe_to_prune {
                let lock_path = self.root.lock_path(id.revision, id.kind, id.key);
                self.roots[root_index].publication_busy = PublishLock::is_busy(&lock_path)?;
            }
            let mut directories = Vec::with_capacity(MAX_ARTIFACT_ROOT_ENTRIES);
            directories.push(CensusDirectory {
                parent: None,
                name: OsString::new(),
            });
            let mut pending = Vec::with_capacity(MAX_ARTIFACT_ROOT_ENTRIES);
            pending.push(0);
            self.tree = Some(ArtifactTreeCursor {
                directories,
                pending,
            });
        }

        let mut tree = self.tree.take().expect("artifact tree initialized");
        let Some(directory) = tree.pending.pop() else {
            self.roots[root_index].fully_censused = true;
            self.phase = CacheCensusPhase::Artifact(root_index + 1);
            return Ok(());
        };
        let root_id = self.roots[root_index].id;
        let root_path = self.root.artifact_root_path(root_id);
        let directory_path = census_directory_path(&root_path, &tree.directories, directory)?;
        for entry in fs::read_dir(&directory_path)? {
            let entry = entry?;
            let metadata = fs::symlink_metadata(entry.path())?;
            let file_type = metadata.file_type();
            if file_type.is_symlink() {
                self.roots[root_index].safe_to_prune = false;
                self.add_product_entry(root_index, 0, false)?;
                if self.overrun.is_some() {
                    break;
                }
                continue;
            }
            if metadata.is_dir() {
                self.add_product_entry(root_index, 0, false)?;
                if self.overrun.is_some() {
                    break;
                }
                let name = entry.file_name();
                if name.len() > MAX_CENSUS_NAME_BYTES
                    || tree.directories.len() == MAX_ARTIFACT_ROOT_ENTRIES
                {
                    self.roots[root_index].safe_to_prune = false;
                    self.foreign_tree = true;
                    self.overrun = Some(CacheCensusOverrun::ArtifactRootEntries);
                    break;
                }
                let child = tree.directories.len();
                tree.directories.push(CensusDirectory {
                    parent: Some(directory),
                    name,
                });
                tree.pending.push(child);
            } else if metadata.is_file() {
                self.add_product_entry(root_index, metadata.len(), true)?;
                if self.overrun.is_some() {
                    break;
                }
            } else {
                self.roots[root_index].safe_to_prune = false;
                self.foreign_tree = true;
                break;
            }
        }
        if self.overrun.is_some() || self.foreign_tree {
            self.phase = CacheCensusPhase::Complete;
        } else if tree.pending.is_empty() {
            self.roots[root_index].fully_censused = true;
            self.phase = CacheCensusPhase::Artifact(root_index + 1);
        } else {
            self.tree = Some(tree);
        }
        Ok(())
    }

    fn add_product_entry(
        &mut self,
        root_index: usize,
        bytes: u64,
        regular_file: bool,
    ) -> Result<(), MapCacheError> {
        self.entries = self
            .entries
            .checked_add(1)
            .ok_or(MapCacheError::Map(MapError::Limit(
                "map-cache census entries",
            )))?;
        self.product_entries = self
            .product_entries
            .checked_add(1)
            .ok_or(MapCacheError::Map(MapError::Limit(
                "map-cache census entries",
            )))?;
        let summary = &mut self.roots[root_index];
        summary.entries =
            summary
                .entries
                .checked_add(1)
                .ok_or(MapCacheError::Map(MapError::Limit(
                    "map-cache census entries",
                )))?;
        if regular_file {
            self.bytes =
                self.bytes
                    .checked_add(bytes)
                    .ok_or(MapCacheError::Map(MapError::Limit(
                        "map-cache census bytes",
                    )))?;
            summary.bytes =
                summary
                    .bytes
                    .checked_add(bytes)
                    .ok_or(MapCacheError::Map(MapError::Limit(
                        "map-cache census bytes",
                    )))?;
        }
        if summary.entries > MAX_ARTIFACT_ROOT_ENTRIES
            || self.product_entries > MAX_CACHE_SCAN_ENTRIES
        {
            self.overrun = Some(CacheCensusOverrun::ArtifactRootEntries);
        }
        Ok(())
    }
}

fn census_directory_path(
    root: &Path,
    directories: &[CensusDirectory],
    mut index: usize,
) -> Result<PathBuf, MapCacheError> {
    let mut components = Vec::with_capacity(directories.len().min(MAX_ARTIFACT_ROOT_ENTRIES));
    while let Some(parent) = directories[index].parent {
        components.push(index);
        index = parent;
    }
    let mut path = root.to_path_buf();
    for component in components.into_iter().rev() {
        path.push(&directories[component].name);
        if path.as_os_str().len() > MAX_CENSUS_PATH_BYTES {
            return Err(MapCacheError::Map(MapError::Limit(
                "map-cache census path bytes",
            )));
        }
    }
    Ok(path)
}

#[derive(Debug)]
struct PruneTreeCursor {
    directories: Vec<CensusDirectory>,
    pending: Vec<usize>,
    postorder: Vec<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PruneAdvance {
    Pending,
    Complete,
    Stale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrunePhase {
    Marker,
    Scan,
    RemoveDirectories,
    Finish,
    Complete,
}

struct PruneCursor {
    root: MapCacheRoot,
    id: ArtifactRootId,
    identity: ArtifactIdentity,
    expected_generation: u64,
    marker_created: bool,
    phase: PrunePhase,
    tree: Option<PruneTreeCursor>,
    visited_entries: usize,
}

impl PruneCursor {
    fn new(
        root: MapCacheRoot,
        summary: &ArtifactRootSummary,
        generation: u64,
    ) -> Result<Self, MapCacheError> {
        if !summary.fully_censused
            || !summary.safe_to_prune
            || summary.identity.is_none()
            || summary.entries > MAX_ARTIFACT_ROOT_ENTRIES
        {
            return Err(MapCacheError::Map(MapError::Path));
        }
        Ok(Self {
            root,
            id: summary.id,
            identity: summary.identity.ok_or(MapCacheError::Map(MapError::Path))?,
            expected_generation: generation,
            marker_created: summary.marked_for_prune,
            phase: PrunePhase::Marker,
            tree: None,
            visited_entries: 0,
        })
    }

    fn reset_tree(&mut self) {
        self.tree = None;
        self.visited_entries = 1;
        self.phase = PrunePhase::Scan;
    }

    fn advance(&mut self) -> Result<PruneAdvance, MapCacheError> {
        if self.phase == PrunePhase::Complete {
            return Ok(PruneAdvance::Complete);
        }
        let Some(mut capacity) = self.root.try_acquire_capacity()? else {
            return Ok(PruneAdvance::Pending);
        };
        let generation = capacity.generation();
        if generation != self.expected_generation {
            if !self.marker_created {
                return Ok(PruneAdvance::Stale);
            }
            self.expected_generation = generation;
            self.reset_tree();
        }
        let lock_path = self
            .root
            .lock_path(self.id.revision, self.id.kind, self.id.key);
        let Some(_key_lock) = PublishLock::try_acquire(&lock_path)? else {
            // A new ready owner can arrive after the census. Recheck all
            // candidates instead of waiting on that owner's lifetime.
            return Ok(if self.marker_created {
                PruneAdvance::Pending
            } else {
                PruneAdvance::Stale
            });
        };
        let marker_path = self.root.prune_marker_path(self.id);
        match read_prune_marker(&marker_path, self.id.revision, self.id.kind, self.id.key)? {
            Some((id, identity)) if id == self.id && identity == self.identity => {
                self.marker_created = true;
            }
            Some(_) => return Err(MapCacheError::Map(MapError::Path)),
            None if self.marker_created => return Ok(PruneAdvance::Stale),
            None => {
                let root_path = self.root.artifact_root_path(self.id);
                let metadata = match fs::symlink_metadata(&root_path) {
                    Ok(metadata) => metadata,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        return Ok(PruneAdvance::Stale)
                    }
                    Err(error) => return Err(error.into()),
                };
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(MapCacheError::Map(MapError::Path));
                }
                if read_root_identity(&root_path, self.id)? != Some(self.identity) {
                    return Err(MapCacheError::Map(MapError::Path));
                }
                let bytes = serde_json::to_vec(&PruneMarker {
                    schema: PRUNE_MARKER_SCHEMA,
                    identity: self.identity,
                    partial: self.id.partial,
                })
                .map_err(MapError::Schema)?;
                if bytes.len() > 256 {
                    return Err(MapCacheError::Map(MapError::Limit(
                        "map-cache prune marker",
                    )));
                }
                self.expected_generation = capacity.bump()?;
                atomic_write(&marker_path, &bytes)?;
                self.marker_created = true;
                self.phase = PrunePhase::Scan;
                return Ok(PruneAdvance::Pending);
            }
        }
        match self.phase {
            PrunePhase::Marker => {
                self.phase = PrunePhase::Scan;
                Ok(PruneAdvance::Pending)
            }
            PrunePhase::Scan => self.scan_one_directory(&mut capacity),
            PrunePhase::RemoveDirectories => self.remove_one_directory(&mut capacity),
            PrunePhase::Finish => self.remove_marker(&mut capacity),
            PrunePhase::Complete => Ok(PruneAdvance::Complete),
        }
    }

    fn scan_one_directory(
        &mut self,
        capacity: &mut CapacityLock,
    ) -> Result<PruneAdvance, MapCacheError> {
        let root_path = self.root.artifact_root_path(self.id);
        let root_metadata = match fs::symlink_metadata(&root_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.phase = PrunePhase::Finish;
                return Ok(PruneAdvance::Pending);
            }
            Err(error) => return Err(error.into()),
        };
        if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
            return Err(MapCacheError::Map(MapError::Path));
        }
        if self.tree.is_none() {
            self.visited_entries = 1;
            self.tree = Some(PruneTreeCursor {
                directories: vec![CensusDirectory {
                    parent: None,
                    name: OsString::new(),
                }],
                pending: vec![0],
                postorder: Vec::with_capacity(MAX_ARTIFACT_ROOT_ENTRIES),
            });
        }
        let mut tree = self.tree.take().expect("prune tree initialized");
        let Some(directory) = tree.pending.pop() else {
            self.tree = Some(tree);
            self.phase = PrunePhase::RemoveDirectories;
            return Ok(PruneAdvance::Pending);
        };
        let path = census_directory_path(&root_path, &tree.directories, directory)?;
        let mut files = Vec::with_capacity(MAX_ARTIFACT_ROOT_ENTRIES);
        let mut child_directories = Vec::with_capacity(MAX_ARTIFACT_ROOT_ENTRIES);
        for entry in fs::read_dir(&path)? {
            let entry = entry?;
            self.visited_entries =
                self.visited_entries
                    .checked_add(1)
                    .ok_or(MapCacheError::Map(MapError::Limit(
                        "map-cache prune entries",
                    )))?;
            if self.visited_entries > MAX_ARTIFACT_ROOT_ENTRIES {
                return Err(MapCacheError::Map(MapError::Limit(
                    "map-cache prune entries",
                )));
            }
            let metadata = fs::symlink_metadata(entry.path())?;
            if metadata.file_type().is_symlink() {
                return Err(MapCacheError::Map(MapError::Path));
            }
            let name = entry.file_name();
            if name.len() > MAX_CENSUS_NAME_BYTES {
                return Err(MapCacheError::Map(MapError::Path));
            }
            if metadata.is_dir() {
                child_directories.push(name);
            } else if metadata.is_file() {
                files.push(name);
            } else {
                return Err(MapCacheError::Map(MapError::Path));
            }
        }
        if !files.is_empty() {
            self.expected_generation = capacity.bump()?;
            for name in files {
                fs::remove_file(path.join(name))?;
            }
            sync_directory(&path)?;
        }
        tree.postorder.push(directory);
        for name in child_directories {
            if tree.directories.len() == MAX_ARTIFACT_ROOT_ENTRIES {
                return Err(MapCacheError::Map(MapError::Limit(
                    "map-cache prune entries",
                )));
            }
            let child = tree.directories.len();
            tree.directories.push(CensusDirectory {
                parent: Some(directory),
                name,
            });
            tree.pending.push(child);
        }
        if tree.pending.is_empty() {
            self.phase = PrunePhase::RemoveDirectories;
        }
        self.tree = Some(tree);
        Ok(PruneAdvance::Pending)
    }

    fn remove_one_directory(
        &mut self,
        capacity: &mut CapacityLock,
    ) -> Result<PruneAdvance, MapCacheError> {
        let root_path = self.root.artifact_root_path(self.id);
        let (directory, path) = {
            let tree = self.tree.as_mut().expect("prune tree initialized");
            let Some(directory) = tree.postorder.pop() else {
                self.phase = PrunePhase::Finish;
                return Ok(PruneAdvance::Pending);
            };
            (
                directory,
                census_directory_path(&root_path, &tree.directories, directory)?,
            )
        };
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(MapCacheError::Map(MapError::Path));
        }
        self.expected_generation = capacity.bump()?;
        fs::remove_dir(&path)?;
        if let Some(parent) = path.parent() {
            sync_directory(parent)?;
        }
        if directory == 0 {
            self.tree = None;
            self.phase = PrunePhase::Finish;
        }
        Ok(PruneAdvance::Pending)
    }

    fn remove_marker(
        &mut self,
        capacity: &mut CapacityLock,
    ) -> Result<PruneAdvance, MapCacheError> {
        let marker_path = self.root.prune_marker_path(self.id);
        let Some((id, identity)) =
            read_prune_marker(&marker_path, self.id.revision, self.id.kind, self.id.key)?
        else {
            return Err(MapCacheError::Map(MapError::Path));
        };
        if id != self.id || identity != self.identity {
            return Err(MapCacheError::Map(MapError::Path));
        }
        let root_path = self.root.artifact_root_path(self.id);
        match fs::symlink_metadata(&root_path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Ok(_) => return Ok(PruneAdvance::Pending),
            Err(error) => return Err(error.into()),
        }
        self.expected_generation = capacity.bump()?;
        fs::remove_file(&marker_path)?;
        sync_directory(
            marker_path
                .parent()
                .ok_or(MapCacheError::Map(MapError::Path))?,
        )?;
        self.phase = PrunePhase::Complete;
        Ok(PruneAdvance::Complete)
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
    /// Read and verify one listed PNG into immutable shared custody.
    ///
    /// An unlisted tile is absent (`None`); a listed tile that cannot be read
    /// or fails its manifest receipt is an error.
    pub fn read_tile_arc(
        &self,
        key: nav::map::spatial::TileKey,
    ) -> Result<Option<Arc<[u8]>>, MapCacheError> {
        Ok(self.inner.read_tile_arc(key)?)
    }
}

/// Runs `op` once. WorkAdmission-owned map work retries a transient Windows
/// refusal only on a later ordinary demand or capacity publication; it never
/// sleeps inside a worker dispatch.
fn run_once<T>(op: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    op()
}

/// Shared lease on an accepted ready directory; a valid prune marker denies it.
fn ready_lease(
    lock_path: &Path,
    ready: &Path,
    marker_path: &Path,
    root_id: ArtifactRootId,
    expected_identity: ArtifactIdentity,
) -> Result<Option<PublishLock>, MapCacheError> {
    // An absent ready root needs no lease and must not create a key file
    // or its parent directories during a read-only terrain probe.
    if !real_directory(ready)? {
        return Ok(None);
    }
    let Some(lease) = PublishLock::try_acquire_shared(lock_path)? else {
        return Ok(None);
    };
    if let Some((_, marker_identity)) =
        read_prune_marker(marker_path, root_id.revision, root_id.kind, root_id.key)?
    {
        if marker_identity != expected_identity || !identity_matches_root(marker_identity, root_id)
        {
            return Err(MapCacheError::Map(MapError::Path));
        }
        return Ok(None);
    }
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

/// Immutable map input binding.  A runtime binding clones the owning
/// `Arc<PreparedRuntimeCache>`; no worker stores only `jag_dir` or
/// `snapshot_dir` after a profile can be rebound.  Release packaging binds an
/// offline client snapshot instead ([`Self::offline`]); no profile exists to
/// rebind there, and its caller owns those directories for the whole bake.
#[derive(Clone)]
pub struct PreparedMapInput {
    revision: u16,
    content: Digest,
    source: MapInputSource,
}

#[derive(Clone)]
enum MapInputSource {
    Prepared(Arc<PreparedRuntimeCache>),
    Offline(Arc<OfflineSnapshot>),
}

struct OfflineSnapshot {
    jag_dir: PathBuf,
    snapshot_dir: PathBuf,
}

impl fmt::Debug for PreparedMapInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedMapInput")
            .field("revision", &self.revision)
            .field("content", &self.content)
            .field(
                "source",
                &match &self.source {
                    MapInputSource::Prepared(_) => "Arc<PreparedRuntimeCache>",
                    MapInputSource::Offline(_) => "offline snapshot",
                },
            )
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
            source: MapInputSource::Prepared(prepared),
        })
    }

    /// An offline client cache, as the nav build reads it: the jag archives
    /// in `jag_dir` and the decoded snapshot under `snapshot_root`, keyed by
    /// the version of `jag_dir/versionlist`.  The content identity is
    /// computed from those files exactly as runtime preparation computes it,
    /// never declared.
    pub fn offline(
        revision: u16,
        jag_dir: &Path,
        snapshot_root: &Path,
    ) -> Result<Self, MapCacheError> {
        let versionlist = fs::read(jag_dir.join("versionlist"))?;
        let snapshot_dir = snapshot_root.join(client::unpack::version_hash(&versionlist));
        // The raster reads the jag directory's versionlist and the identity
        // the snapshot's: they must be one file.
        if fs::read(snapshot_dir.join("versionlist"))? != versionlist {
            return Err(MapCacheError::Message(format!(
                "snapshot {} does not hold the versionlist of {}",
                snapshot_dir.display(),
                jag_dir.display()
            )));
        }
        let identity = client::content_identity::compute_decoded_content_identity(
            revision,
            jag_dir,
            &snapshot_dir,
        )
        .map_err(|error| MapCacheError::Message(format!("decoded content identity: {error}")))?;
        Ok(Self::with_snapshot(
            revision,
            Digest(identity.content_id),
            jag_dir.to_path_buf(),
            snapshot_dir,
        ))
    }

    /// An offline snapshot whose content identity the caller already
    /// established ([`Self::offline`], or a synthetic test snapshot).
    pub(crate) fn with_snapshot(
        revision: u16,
        content: Digest,
        jag_dir: PathBuf,
        snapshot_dir: PathBuf,
    ) -> Self {
        Self {
            revision,
            content,
            source: MapInputSource::Offline(Arc::new(OfflineSnapshot {
                jag_dir,
                snapshot_dir,
            })),
        }
    }

    pub fn revision(&self) -> u16 {
        self.revision
    }
    pub fn content(&self) -> Digest {
        self.content
    }
    pub fn jag_dir(&self) -> &Path {
        match &self.source {
            MapInputSource::Prepared(prepared) => &prepared.jag_dir,
            MapInputSource::Offline(snapshot) => &snapshot.jag_dir,
        }
    }
    pub fn snapshot_dir(&self) -> &Path {
        match &self.source {
            MapInputSource::Prepared(prepared) => &prepared.snapshot_dir,
            MapInputSource::Offline(snapshot) => &snapshot.snapshot_dir,
        }
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
    /// Catalogue plus terrain, baking terrain locally when it is not ready.
    Images,
    /// Catalogue plus already-published terrain only: the worker derives a
    /// missing catalogue but never starts a terrain bake. When the terrain is
    /// gone by the image stage the demand settles Ready without images.
    ReadyImages,
}

impl MapDemand {
    fn wants_images(self) -> bool {
        matches!(self, Self::Images | Self::ReadyImages)
    }

    /// Worker demand flag: what the image stage may do.
    fn flag(self) -> usize {
        match self {
            Self::CatalogueOnly => DEMAND_CATALOGUE,
            Self::Images => DEMAND_BAKE_IMAGES,
            Self::ReadyImages => DEMAND_ADOPT_IMAGES,
        }
    }
}

const DEMAND_CATALOGUE: usize = 0;
const DEMAND_BAKE_IMAGES: usize = 1;
const DEMAND_ADOPT_IMAGES: usize = 2;

fn upgrade_demand(flag: &AtomicUsize, demand: MapDemand) {
    match demand {
        MapDemand::Images => flag.store(DEMAND_BAKE_IMAGES, AtomicOrdering::Release),
        MapDemand::ReadyImages => {
            let _ = flag.compare_exchange(
                DEMAND_CATALOGUE,
                DEMAND_ADOPT_IMAGES,
                AtomicOrdering::AcqRel,
                AtomicOrdering::Acquire,
            );
        }
        MapDemand::CatalogueOnly => {}
    }
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
    partial_dir: Arc<PathBuf>,
}

impl fmt::Debug for BakeRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BakeRequest")
            .field("artifact", &self.artifact)
            .field("descriptor", &self.descriptor)
            .field("partial_dir", &self.partial_dir)
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
    pub fn partial_dir(&self) -> &Path {
        self.partial_dir.as_path()
    }
    pub(crate) fn partial_dir_arc(&self) -> &Arc<PathBuf> {
        &self.partial_dir
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

fn bake_request(
    root: &MapCacheRoot,
    descriptor: MapProfileDescriptor,
    artifact: ArtifactKind,
) -> Result<BakeRequest, MapCacheError> {
    let key = match artifact {
        ArtifactKind::Catalogue => descriptor.catalogue_key()?,
        ArtifactKind::Images => descriptor.image_key()?,
    };
    Ok(BakeRequest {
        artifact,
        partial_dir: Arc::new(root.partial_dir(descriptor.revision(), artifact, key)),
        descriptor,
    })
}

/// B owns format decoding/raster policy; D owns demand, persistence and
/// publication. A cursor performs bounded units and retains only resumable
/// state; the host records each verified unit and owns the final publication.
pub trait MapBakeProducer: Send + Sync + 'static {
    /// Conservative retained/scratch/result reservation from identities and
    /// path metadata only. Must not open files or construct a cursor.
    fn charge(&self, request: &BakeRequest) -> Result<WorkCharge, MapCacheError>;
    fn cursor(
        &self,
        request: &BakeRequest,
        partial_dir: Arc<PathBuf>,
    ) -> Result<Box<dyn MapBakeCursor>, MapCacheError>;
}

#[derive(Debug, Clone, Copy)]
pub struct MapProgressSnapshot {
    pub stage: MapStage,
    pub completed: u32,
    pub total: Option<u32>,
}

fn normalize_cursor_progress(progress: MapProgressSnapshot) -> MapProgressSnapshot {
    MapProgressSnapshot {
        total: progress.total.filter(|total| *total != 0),
        ..progress
    }
}

fn cursor_progress_bucket(progress: MapProgressSnapshot) -> u32 {
    match progress.total {
        Some(total) if total != 0 => {
            (u64::from(progress.completed.min(total)) * 100 / u64::from(total)) as u32
        }
        _ => progress.completed / MAP_PROGRESS_UNKNOWN_STEP,
    }
}

fn should_publish_cursor_progress(
    previous: Option<MapProgressSnapshot>,
    progress: MapProgressSnapshot,
    final_progress: bool,
) -> bool {
    if final_progress {
        return true;
    }
    let Some(previous) = previous else {
        return true;
    };
    previous.stage != progress.stage
        || previous.total != progress.total
        || cursor_progress_bucket(previous) != cursor_progress_bucket(progress)
}

fn cursor_progress_boundary(
    previous: Option<MapProgressSnapshot>,
    progress: MapProgressSnapshot,
    final_progress: bool,
) -> bool {
    final_progress
        || previous.is_none_or(|previous| {
            previous.stage != progress.stage || previous.total != progress.total
        })
}

pub trait MapBakeCursor: Send + 'static {
    /// Exact artifact plan once all incremental preflight is complete.
    fn plan(&self) -> Option<BakePlan>;
    /// The writer remains absent only while a cursor is still discovering its plan.
    /// Without a writer, preflight uses only immutable inputs, not cache output.
    /// Yield when the plan becomes available so the job can acquire cache ownership.
    fn step(
        &mut self,
        context: &mut api::work::StepContext<'_, api::work::MapBakePhase>,
        writer: Option<&mut BakeWriter>,
    ) -> Result<MapBakeCursorStep, MapCacheError>;
    fn progress(&self) -> MapProgressSnapshot;
    /// Receipts completed or adopted by the cursor since the preceding step.
    /// The job persists full image batches and flushes the remainder at completion.
    fn take_completed_units(&mut self) -> Vec<CompletedUnit>;
}

pub enum MapBakeCursorOutput {
    Catalogue(ClientPois),
    Images(ImageManifest),
}

pub enum MapBakeCursorStep {
    Yielded,
    Completed(MapBakeCursorOutput),
    Cancelled,
    Rejected(MapCacheError),
}

#[derive(Debug, Clone, Copy)]
struct MapBakeInput {
    key: JobKey,
    generation: u64,
    artifact: ArtifactKind,
    charge: WorkCharge,
}

struct MapBakeResult {
    key: JobKey,
    generation: u64,
    artifact: ArtifactKind,
    ready: ReadyMap,
}

/// Add host-owned job, path, checkpoint and file-backed ready-state costs to
/// the navigation cursor's independently measured reservation.
fn host_map_bake_charge(
    request: &BakeRequest,
    producer: WorkCharge,
) -> Result<WorkCharge, MapCacheError> {
    let path_bytes = request
        .partial_dir_arc()
        .capacity()
        .checked_add(MAP_BAKE_PATH_SUFFIX_RESERVE_BYTES)
        .and_then(|bytes| bytes.checked_mul(MAP_BAKE_PATH_OWNER_COUNT))
        .ok_or(MapCacheError::Map(MapError::Limit("map-bake work charge")))?;
    let checkpoint_bytes = MAX_IMAGE_TILES
        .checked_mul(std::mem::size_of::<CompletedUnit>())
        .ok_or(MapCacheError::Map(MapError::Limit("map-bake work charge")))?;
    let census_input_bytes = [
        (std::mem::size_of::<u16>(), MAX_CACHE_ARTIFACT_ROOTS),
        (
            std::mem::size_of::<(u16, ArtifactKind)>(),
            2 * MAX_CACHE_ARTIFACT_ROOTS,
        ),
        (
            std::mem::size_of::<KindDirectoryEntry>(),
            MAX_ARTIFACT_ROOT_ENTRIES,
        ),
        (
            std::mem::size_of::<ArtifactRootSummary>(),
            MAX_CACHE_ARTIFACT_ROOTS,
        ),
        (
            std::mem::size_of::<CensusDirectory>(),
            2 * MAX_ARTIFACT_ROOT_ENTRIES,
        ),
        (std::mem::size_of::<usize>(), 4 * MAX_ARTIFACT_ROOT_ENTRIES),
        (
            std::mem::size_of::<OsString>(),
            2 * MAX_ARTIFACT_ROOT_ENTRIES,
        ),
        (
            1,
            MAX_CENSUS_NAME_BYTES * MAX_ARTIFACT_ROOT_ENTRIES + 2 * MAX_CENSUS_PATH_BYTES,
        ),
    ]
    .into_iter()
    .try_fold(0usize, |sum, (size, count)| {
        sum.checked_add(size.checked_mul(count)?)
    })
    .ok_or(MapCacheError::Map(MapError::Limit("map-bake work charge")))?;
    let ready_image_bytes = MAX_IMAGE_TILES
        .checked_mul(std::mem::size_of::<TileReceipt>())
        .ok_or(MapCacheError::Map(MapError::Limit("map-bake work charge")))?;
    let input_bytes = [
        std::mem::size_of::<MapBakeInput>(),
        std::mem::size_of::<MapBakeJob>(),
        std::mem::size_of::<ActiveJob>(),
        std::mem::size_of::<Checkpoint>(),
        // Decoded rows can retain <2*MAX capacity while verified rows grow to MAX.
        checkpoint_bytes
            .checked_mul(3)
            .ok_or(MapCacheError::Map(MapError::Limit("map-bake work charge")))?,
        path_bytes,
        census_input_bytes,
        MAP_BAKE_PATH_NODE_BYTES * MAP_BAKE_REQUIRED_PATH_NODE_COUNT,
    ]
    .into_iter()
    .try_fold(0usize, |sum, bytes| sum.checked_add(bytes))
    .and_then(|host| producer.input_bytes().checked_add(host))
    .ok_or(MapCacheError::Map(MapError::Limit("map-bake work charge")))?;
    let identity_read_scratch_bytes = MAX_JSON_BYTES
        .checked_add(1)
        .ok_or(MapCacheError::Map(MapError::Limit("map-bake work charge")))?;
    let census_scratch_bytes = [
        64 * 1024,
        256,
        2 * MAX_CENSUS_PATH_BYTES + MAP_BAKE_PATH_SUFFIX_RESERVE_BYTES,
        identity_read_scratch_bytes,
    ]
    .into_iter()
    .try_fold(0usize, |sum, bytes| sum.checked_add(bytes))
    .ok_or(MapCacheError::Map(MapError::Limit("map-bake work charge")))?;
    let verification_scratch_bytes = MAX_JSON_BYTES
        .max(MAX_PNG_BYTES as usize)
        .checked_add(1)
        .ok_or(MapCacheError::Map(MapError::Limit("map-bake work charge")))?;
    let serialization_scratch_bytes = MAX_JSON_BYTES
        .checked_mul(2)
        .and_then(|bytes| bytes.checked_add(1))
        .ok_or(MapCacheError::Map(MapError::Limit("map-bake work charge")))?;
    // Census reads, resumed-payload verification, producer cursor work, and
    // JSON finalization occupy separate stages. The cursor is discarded
    // before finalization so its scratch cannot overlap those JSON buffers.
    let scratch_bytes = [
        census_scratch_bytes,
        verification_scratch_bytes,
        producer.scratch_bytes(),
        serialization_scratch_bytes,
    ]
    .into_iter()
    .max()
    .ok_or(MapCacheError::Map(MapError::Limit("map-bake work charge")))?;
    let result_bytes = [
        std::mem::size_of::<MapBakeResult>(),
        std::mem::size_of::<NavReadyCatalogue>(),
        std::mem::size_of::<NavReadyImages>(),
        std::mem::size_of::<ReadyMap>(),
        std::mem::size_of::<ReadyCatalogue>(),
        std::mem::size_of::<ReadyImages>(),
        MAX_JSON_BYTES,
        ready_image_bytes,
    ]
    .into_iter()
    .try_fold(0usize, |sum, bytes| sum.checked_add(bytes))
    .and_then(|host| producer.result_bytes().checked_add(host))
    .ok_or(MapCacheError::Map(MapError::Limit("map-bake work charge")))?;
    let operation_handles = producer
        .operation_handles()
        .checked_add(4)
        .ok_or(MapCacheError::Map(MapError::Limit("map-bake work charge")))?
        .max(4);
    Ok(WorkCharge::new(
        input_bytes,
        scratch_bytes,
        result_bytes,
        operation_handles,
    ))
}

struct MapBakeSpec;

impl WorkFamilySpec for MapBakeSpec {
    const FAMILY: WorkFamily = WorkFamily::MapBake;
    type Input = MapBakeInput;
    type Output = MapBakeResult;
    type Error = MapCacheError;
    type Phase = MapBakePhase;

    fn revision(input: &Self::Input) -> InputRevision {
        InputRevision::new(input.generation)
    }

    fn charge(input: &Self::Input) -> WorkCharge {
        input.charge
    }

    fn map_cancellation(reason: Cancellation) -> Cancellation {
        reason
    }
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
#[derive(Debug, Clone, Copy)]
struct CacheUsage {
    bytes: u64,
    entries: u64,
    product_entries: u64,
    root_count: usize,
    root_known: bool,
    artifact_entries: usize,
}

impl CacheUsage {
    fn for_root(census: &CacheCensusResult, id: ArtifactRootId) -> Self {
        let root = census.roots.iter().find(|root| root.id == id);
        Self {
            entries: census.entries,
            bytes: census.bytes,
            product_entries: census.product_entries,
            root_count: census.roots.len(),
            root_known: root.is_some(),
            artifact_entries: root
                .filter(|root| root.root_exists)
                .map_or(0, |root| root.entries),
        }
    }
}

struct CheckpointVerification {
    next: usize,
    completed: Vec<CompletedUnit>,
    bytes: u64,
    payload: Vec<u8>,
}

pub struct BakeWriter {
    partial: Arc<PathBuf>,
    identity: ArtifactIdentity,
    checkpoint: Checkpoint,
    cancel: Arc<AtomicBool>,
    progress: Arc<dyn Fn(MapProgress) + Send + Sync>,
    bytes: u64,
    cache_bytes: u64,
    cache_entries: u64,
    cache_product_entries: u64,
    artifact_entries: usize,
    checkpoint_dirty: bool,
    pending_image_directories: [u32; MAP_PLANES as usize],
    pending_image_plane_parents: u8,
    pending_image_terrain_parent: bool,
    pending_image_tiles: usize,
    verification: Option<CheckpointVerification>,
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
    #[cfg(test)]
    fn open(
        root: MapCacheRoot,
        request: &BakeRequest,
        plan: BakePlan,
        cancel: Arc<AtomicBool>,
        progress: Arc<dyn Fn(MapProgress) + Send + Sync>,
    ) -> Result<Self, MapCacheError> {
        let identity = request.expected_identity();
        let key = identity.key()?;
        let mut capacity = root.try_acquire_capacity()?.ok_or(MapCacheError::Busy)?;
        let mut changed = root.ensure_kind_dir(request.descriptor.revision(), request.artifact)?;
        changed |=
            root.ensure_key_lock_file(request.descriptor.revision(), request.artifact, key)?;
        if changed {
            capacity.bump()?;
        }
        drop(capacity);
        let lock_path = root.lock_path(request.descriptor.revision(), request.artifact, key);
        let _key = PublishLock::try_acquire(&lock_path)?.ok_or(MapCacheError::Busy)?;
        let mut cursor = CacheCensusCursor::new(root.clone());
        let census = loop {
            if let Some(census) = cursor.advance()? {
                break census;
            }
        };
        let usage = CacheUsage::for_root(
            &census,
            ArtifactRootId {
                revision: request.descriptor.revision(),
                kind: request.artifact,
                key,
                partial: true,
            },
        );
        let mut writer = Self::open_at(
            &root,
            Arc::clone(&request.partial_dir),
            request,
            plan,
            cancel,
            progress,
            usage,
        )?;
        while writer.verify_next_checkpoint_unit()? {}
        writer.persist_checkpoint()?;
        Ok(writer)
    }

    fn open_at(
        root: &MapCacheRoot,
        partial: Arc<PathBuf>,
        request: &BakeRequest,
        plan: BakePlan,
        cancel: Arc<AtomicBool>,
        progress: Arc<dyn Fn(MapProgress) + Send + Sync>,
        mut usage: CacheUsage,
    ) -> Result<Self, MapCacheError> {
        let identity = request.expected_identity();
        let partial_expected = root.partial_dir(
            request.descriptor.revision(),
            request.artifact,
            identity.key()?,
        );
        if partial.as_path() != partial_expected.as_path() {
            return Err(MapCacheError::Map(MapError::Identity));
        }
        let partial_exists = match fs::symlink_metadata(partial.as_path()) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(MapCacheError::Map(MapError::Path));
            }
            Ok(_) => true,
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => return Err(error.into()),
        };
        if partial_exists != (usage.artifact_entries != 0)
            || (partial_exists && !usage.root_known)
            || (!partial_exists && usage.root_known)
        {
            return Err(MapCacheError::Map(MapError::Path));
        }
        let checkpoint = if partial_exists {
            read_regular_bounded(&partial.join("checkpoint.json"), MAX_JSON_BYTES)?
                .and_then(|bytes| Checkpoint::decode(&bytes, identity).ok())
                .filter(|checkpoint| checkpoint.planned_units == plan.planned_units)
        } else {
            None
        };
        let checkpoint_dirty = checkpoint.is_none();
        if !partial_exists {
            if usage.root_count == MAX_CACHE_ARTIFACT_ROOTS {
                return Err(MapCacheError::Map(MapError::Limit(
                    "map-cache root summary capacity",
                )));
            }
            let next_product_entries =
                usage
                    .product_entries
                    .checked_add(1)
                    .ok_or(MapCacheError::Map(MapError::Limit(
                        "map-cache census entries",
                    )))?;
            let next_entries =
                usage
                    .entries
                    .checked_add(1)
                    .ok_or(MapCacheError::Map(MapError::Limit(
                        "map-cache census entries",
                    )))?;
            if next_product_entries > MAX_CACHE_SCAN_ENTRIES
                || next_entries > MAX_CACHE_SCAN_ENTRIES
            {
                return Err(MapCacheError::Map(MapError::Limit(
                    "map-cache census entries",
                )));
            }
            fs::create_dir(partial.as_path())?;
            sync_directory(partial.parent().ok_or(MapCacheError::Map(MapError::Path))?)?;
            usage.entries = next_entries;
            usage.product_entries = next_product_entries;
            usage.root_count += 1;
            usage.artifact_entries = 1;
        }
        if usage.artifact_entries > MAX_ARTIFACT_ROOT_ENTRIES {
            return Err(MapCacheError::Map(MapError::Limit(
                "map-cache artifact entries",
            )));
        }
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
            .try_fold(0u64, |sum, unit| {
                sum.checked_add(u64::from(unit.payload.bytes))
            })
            .ok_or(MapCacheError::Map(MapError::Limit("artifact bytes")))?;
        if bytes > max_artifact_bytes(request.artifact) {
            return Err(MapCacheError::Map(MapError::Limit("artifact bytes")));
        }
        let verification =
            (!checkpoint.completed.as_slice().is_empty()).then(|| CheckpointVerification {
                next: 0,
                completed: Vec::with_capacity(checkpoint.completed.as_slice().len()),
                bytes: 0,
                payload: Vec::with_capacity(MAX_JSON_BYTES.max(MAX_PNG_BYTES as usize) + 1),
            });
        let writer = Self {
            partial,
            identity,
            checkpoint,
            cancel,
            progress,
            bytes,
            cache_bytes: usage.bytes,
            cache_entries: usage.entries,
            cache_product_entries: usage.product_entries,
            artifact_entries: usage.artifact_entries,
            checkpoint_dirty,
            pending_image_directories: [0; MAP_PLANES as usize],
            pending_image_plane_parents: 0,
            pending_image_terrain_parent: false,
            pending_image_tiles: 0,
            verification,
        };
        Ok(writer)
    }
    /// Verify at most one resumed unit before exposing receipts to the cursor.
    /// Corrupt or absent owned payloads remain work to regenerate.
    fn verify_next_checkpoint_unit(&mut self) -> Result<bool, MapCacheError> {
        let Some(mut verification) = self.verification.take() else {
            return Ok(false);
        };
        let unit = self.checkpoint.completed.as_slice()[verification.next];
        let limit = match unit.key {
            UnitKey::ClientPois => MAX_JSON_BYTES,
            UnitKey::Terrain { .. } => MAX_PNG_BYTES as usize,
        };
        let path = self.partial.join(unit_relative_path(unit.key)?);
        let existing_bytes = regular_file_length(&path)?;
        let valid = match existing_bytes {
            Some(length) if length <= limit as u64 => {
                verification.payload.clear();
                File::open(&path)?
                    .take(limit as u64 + 1)
                    .read_to_end(&mut verification.payload)?;
                match unit
                    .payload
                    .verify(&verification.payload, limit as u32)
                    .map_err(MapCacheError::from)
                    .and_then(|()| self.validate_unit(unit.key, &verification.payload))
                {
                    Ok(()) => true,
                    Err(MapCacheError::Map(_)) => false,
                    Err(error) => return Err(error),
                }
            }
            _ => false,
        };
        if valid {
            verification.completed.push(unit);
            verification.bytes += u64::from(unit.payload.bytes);
        } else {
            self.checkpoint_dirty = true;
            if let Some(length) = existing_bytes {
                // Do not let the producer adopt a well-formed payload whose
                // bytes contradicted its durable checkpoint receipt.
                let cache_bytes = self
                    .cache_bytes
                    .checked_sub(length)
                    .ok_or(MapCacheError::Map(MapError::Path))?;
                let cache_entries = self
                    .cache_entries
                    .checked_sub(1)
                    .ok_or(MapCacheError::Map(MapError::Path))?;
                let product_entries = self
                    .cache_product_entries
                    .checked_sub(1)
                    .ok_or(MapCacheError::Map(MapError::Path))?;
                let artifact_entries = self
                    .artifact_entries
                    .checked_sub(1)
                    .ok_or(MapCacheError::Map(MapError::Path))?;
                fs::remove_file(&path)?;
                sync_directory(path.parent().ok_or(MapCacheError::Map(MapError::Path))?)?;
                self.cache_bytes = cache_bytes;
                self.cache_entries = cache_entries;
                self.cache_product_entries = product_entries;
                self.artifact_entries = artifact_entries;
            }
        }
        verification.next += 1;
        if verification.next == self.checkpoint.completed.as_slice().len() {
            self.checkpoint.completed = nav::map::Rows::new(verification.completed)?;
            self.bytes = verification.bytes;
        } else {
            self.verification = Some(verification);
        }
        Ok(true)
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
        self.completed_receipt(key).is_some()
    }

    /// Receipt the checkpoint recorded for `key`, if it is complete.
    pub fn completed_receipt(&self, key: UnitKey) -> Option<PayloadReceipt> {
        // Verified completed units remain sorted as holes are regenerated.
        let completed = self.checkpoint.completed.as_slice();
        completed
            .binary_search_by(|unit| unit.key.cmp(&key))
            .ok()
            .map(|index| completed[index].payload)
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

    /// Write one complete output unit. Image receipts share a durable batch;
    /// a verified completed unit needs no second write or raster operation.
    pub fn publish_unit(&mut self, key: UnitKey, payload: &[u8]) -> Result<bool, MapCacheError> {
        let Some((next_bytes, index)) = self.admit_unit(key, payload.len() as u64)? else {
            return Ok(false);
        };
        self.validate_unit(key, payload)?;
        // The catalogue and its checkpoint share this parent. The checkpoint's
        // parent sync commits both renames after both file-data syncs.
        self.write_counted(
            &self.partial.join(unit_relative_path(key)?),
            payload,
            !matches!(key, UnitKey::ClientPois),
        )?;
        let receipt = PayloadReceipt {
            bytes: payload.len() as u32,
            sha256: Digest::of(payload),
        };
        self.complete_unit(key, receipt, next_bytes, index)?;
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

    /// Sync one validated PNG file and retain its receipt until the next
    /// checkpoint batch also makes its directory entry durable.
    pub(crate) fn write_image_tile(
        &mut self,
        key: UnitKey,
        png: &[u8],
    ) -> Result<PayloadReceipt, MapCacheError> {
        let Some((next_bytes, index)) = self.admit_unit(key, png.len() as u64)? else {
            return Err(MapCacheError::Map(MapError::Invalid(
                "image tile is already checkpointed",
            )));
        };
        self.validate_unit(key, png)?;
        self.write_counted(&self.partial.join(unit_relative_path(key)?), png, false)?;
        let receipt = PayloadReceipt {
            bytes: png.len() as u32,
            sha256: Digest::of(png),
        };
        self.complete_unit(key, receipt, next_bytes, index)?;
        Ok(receipt)
    }

    /// Adopt a completed payload already present in the partial artifact.
    /// The cursor has validated its contents; this checks its length and
    /// accounts it in the host checkpoint without rewriting the file.
    pub fn record_unit(
        &mut self,
        key: UnitKey,
        receipt: PayloadReceipt,
    ) -> Result<bool, MapCacheError> {
        let Some((next_bytes, index)) = self.admit_unit(key, u64::from(receipt.bytes))? else {
            return Ok(false);
        };
        if regular_file_length(&self.partial.join(unit_relative_path(key)?))?
            != Some(u64::from(receipt.bytes))
        {
            return Err(MapCacheError::Map(MapError::Invalid(
                "recorded unit length",
            )));
        }
        if matches!(key, UnitKey::Terrain { .. }) {
            File::open(self.partial.join(unit_relative_path(key)?))?.sync_all()?;
        }
        self.complete_unit(key, receipt, next_bytes, index)?;
        Ok(true)
    }

    /// Ordering, per-key and generated-cache limits for one more unit of
    /// `payload_bytes`. `None` when the unit is already checkpointed.
    fn admit_unit(
        &mut self,
        key: UnitKey,
        payload_bytes: u64,
    ) -> Result<Option<(u64, usize)>, MapCacheError> {
        if self.is_cancelled() {
            return Err(MapCacheError::Cancelled);
        }
        let index = match self
            .checkpoint
            .completed
            .as_slice()
            .binary_search_by_key(&key, |unit| unit.key)
        {
            Ok(_) => return Ok(None),
            Err(index) => index,
        };
        let next_bytes = self
            .bytes
            .checked_add(payload_bytes)
            .ok_or(MapCacheError::Map(MapError::Limit("artifact bytes")))?;
        if next_bytes > max_artifact_bytes(self.artifact()) {
            return Err(MapCacheError::Map(MapError::Limit("artifact bytes")));
        }
        Ok(Some((next_bytes, index)))
    }

    fn complete_unit(
        &mut self,
        key: UnitKey,
        receipt: PayloadReceipt,
        next_bytes: u64,
        index: usize,
    ) -> Result<(), MapCacheError> {
        self.checkpoint.completed.insert(
            index,
            CompletedUnit {
                key,
                payload: receipt,
            },
        )?;
        self.bytes = next_bytes;
        self.checkpoint_dirty = true;
        if let UnitKey::Terrain { tile } = key {
            self.pending_image_directories[usize::from(tile.plane)] |= 1 << tile.lod;
            self.pending_image_tiles += 1;
            if self.pending_image_tiles >= CHECKPOINT_BATCH_TILES {
                self.persist_checkpoint()?;
            }
        }
        Ok(())
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

    fn ensure_parent_dirs(&mut self, path: &Path) -> Result<(), MapCacheError> {
        let parent = path.parent().ok_or(MapCacheError::Map(MapError::Path))?;
        let relative = parent
            .strip_prefix(self.partial.as_path())
            .map_err(|_| MapCacheError::Map(MapError::Path))?;
        let mut current = self.partial.as_ref().clone();
        for component in relative.components() {
            match component {
                Component::Normal(name) => current.push(name),
                Component::CurDir => continue,
                _ => return Err(MapCacheError::Map(MapError::Path)),
            }
            match fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                    return Err(MapCacheError::Map(MapError::Path));
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    self.check_new_entry_capacity()?;
                    fs::create_dir(&current)?;
                    self.account_new_entry();
                    self.defer_created_parent(
                        current.parent().ok_or(MapCacheError::Map(MapError::Path))?,
                    )?;
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    fn defer_created_parent(&mut self, parent: &Path) -> Result<(), MapCacheError> {
        let relative = parent
            .strip_prefix(self.partial.as_path())
            .map_err(|_| MapCacheError::Map(MapError::Path))?;
        let mut components = relative.components();
        match (components.next(), components.next(), components.next()) {
            (None, None, None) => {
                // The checkpoint parent sync commits the new terrain directory.
            }
            (Some(Component::Normal(name)), None, None) if name == "terrain" => {
                self.pending_image_terrain_parent = true;
            }
            (Some(Component::Normal(terrain)), Some(Component::Normal(plane)), None)
                if terrain == "terrain" =>
            {
                let bytes = plane.as_encoded_bytes();
                let [b'p', digit] = bytes else {
                    return Err(MapCacheError::Map(MapError::Path));
                };
                let index = digit.wrapping_sub(b'0');
                if index >= MAP_PLANES {
                    return Err(MapCacheError::Map(MapError::Path));
                }
                self.pending_image_plane_parents |= 1 << index;
            }
            _ => return Err(MapCacheError::Map(MapError::Path)),
        }
        Ok(())
    }

    fn check_new_entry_capacity(&self) -> Result<(), MapCacheError> {
        if self.artifact_entries >= MAX_ARTIFACT_ROOT_ENTRIES
            || self.cache_entries >= MAX_CACHE_SCAN_ENTRIES
            || self.cache_product_entries >= MAX_CACHE_SCAN_ENTRIES
        {
            return Err(MapCacheError::Map(MapError::Limit(
                "map-cache artifact entries",
            )));
        }
        Ok(())
    }

    fn account_new_entry(&mut self) {
        self.artifact_entries += 1;
        self.cache_entries += 1;
        self.cache_product_entries += 1;
    }

    fn write_counted(
        &mut self,
        path: &Path,
        bytes: &[u8],
        sync_parent: bool,
    ) -> Result<(), MapCacheError> {
        self.ensure_parent_dirs(path)?;
        let old_bytes = regular_file_length(path)?;
        let temporary_bytes =
            self.cache_bytes
                .checked_add(bytes.len() as u64)
                .ok_or(MapCacheError::Map(MapError::Limit(
                    "generated map cache bytes",
                )))?;
        if temporary_bytes > MAX_GENERATED_CACHE_BYTES {
            return Err(MapCacheError::Map(MapError::Limit(
                "generated map cache bytes",
            )));
        }
        self.check_new_entry_capacity()?;
        let next_cache_bytes = self
            .cache_bytes
            .checked_sub(old_bytes.unwrap_or(0))
            .and_then(|total| total.checked_add(bytes.len() as u64))
            .ok_or(MapCacheError::Map(MapError::Limit(
                "generated map cache bytes",
            )))?;
        if next_cache_bytes > MAX_GENERATED_CACHE_BYTES {
            return Err(MapCacheError::Map(MapError::Limit(
                "generated map cache bytes",
            )));
        }
        atomic_write_file(path, bytes, sync_parent)?;
        self.cache_bytes = next_cache_bytes;
        if old_bytes.is_none() {
            self.account_new_entry();
        }
        Ok(())
    }

    fn sync_image_directories(&mut self) -> Result<(), MapCacheError> {
        if self.pending_image_directories.iter().all(|mask| *mask == 0)
            && self.pending_image_plane_parents == 0
            && !self.pending_image_terrain_parent
        {
            return Ok(());
        }
        let mut path = self.partial.join("terrain");
        for (plane, mask) in self.pending_image_directories.iter_mut().enumerate() {
            if *mask == 0 && self.pending_image_plane_parents & (1 << plane) == 0 {
                continue;
            }
            let plane_name = [b'p', b'0' + plane as u8];
            path.push(std::str::from_utf8(&plane_name).expect("ASCII terrain plane"));
            while *mask != 0 {
                let lod = mask.trailing_zeros() as u8;
                let mut lod_name = [b'l', b'0' + lod / 10, b'0' + lod % 10];
                let name = if lod < 10 {
                    lod_name[1] = lod_name[2];
                    &lod_name[..2]
                } else {
                    &lod_name[..]
                };
                path.push(std::str::from_utf8(name).expect("ASCII terrain LOD"));
                sync_directory(&path)?;
                path.pop();
                *mask &= !(1 << lod);
            }
            if self.pending_image_plane_parents & (1 << plane) != 0 {
                sync_directory(&path)?;
                self.pending_image_plane_parents &= !(1 << plane);
            }
            path.pop();
        }
        if self.pending_image_terrain_parent {
            sync_directory(&path)?;
            self.pending_image_terrain_parent = false;
        }
        Ok(())
    }

    fn persist_checkpoint(&mut self) -> Result<(), MapCacheError> {
        if !self.checkpoint_dirty {
            return Ok(());
        }
        // Each PNG file is synced before complete_unit records its receipt.
        // Sync changed tile directories and new parents from children upward.
        // The checkpoint parent sync also commits the new terrain directory.
        // A queued cancellation can drop the cursor without another step.
        // Treat it like a crash: resume verifies this durable batch, then
        // verifies or regenerates every uncheckpointed tile before publication.
        self.sync_image_directories()?;
        let bytes = self.checkpoint.encode()?;
        self.write_counted(&self.partial.join("checkpoint.json"), &bytes, true)?;
        self.checkpoint_dirty = false;
        self.pending_image_tiles = 0;
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

    fn finish(mut self, output: BakeOutput) -> Result<Arc<PathBuf>, MapCacheError> {
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
        // A catalogue's completion manifest commits its single payload in this
        // invocation. A publishing checkpoint would be immediately superseded.
        // Cancellation and incomplete work still persist their checkpoint above.
        if matches!(self.identity, ArtifactIdentity::Image(_)) {
            self.checkpoint.stage = BakeStage::Publishing;
            self.checkpoint_dirty = true;
            self.persist_checkpoint()?;
        }
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
        self.write_counted(&self.partial.join("manifest.json"), &manifest, true)?;
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

fn regular_file_length(path: &Path) -> Result<Option<u64>, MapCacheError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(MapCacheError::Map(MapError::Path))
        }
        Ok(metadata) => Ok(Some(metadata.len())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
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

fn clear_ready_checkpoint(
    directory: &Path,
    capacity: &mut CapacityLock,
) -> Result<(), MapCacheError> {
    if regular_file_length(&directory.join("checkpoint.json"))?.is_some() {
        capacity.bump()?;
        clear_checkpoint(directory)?;
    }
    Ok(())
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

fn map_stage_from_progress(phase: u16) -> Option<MapStage> {
    match MapBakePhase::ALL.get(usize::from(phase))? {
        MapBakePhase::ReadingCache => Some(MapStage::ReadingCache),
        MapBakePhase::DerivingPois => Some(MapStage::DerivingPois),
        MapBakePhase::BakingPlane => Some(MapStage::BakingPlane),
        MapBakePhase::BuildingZoomLevels => Some(MapStage::BuildingZoomLevels),
        MapBakePhase::Publishing => Some(MapStage::Publishing),
    }
}

/// Cross-process advisory lock guard. The OS releases this lock when the
/// owning process exits, so there is no PID marker or stale-lock takeover.
#[derive(Debug)]
struct PublishLock {
    file: File,
}

impl PublishLock {
    fn open_control(path: &Path) -> Result<File, MapCacheError> {
        match OpenOptions::new().read(true).write(true).open(path) {
            Ok(file) => Ok(file),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                Ok(OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(path)?)
            }
            Err(error) => Err(error.into()),
        }
    }

    fn try_acquire(path: &Path) -> Result<Option<Self>, MapCacheError> {
        let file = Self::open_control(path)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Self { file })),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(error)) => Err(error.into()),
        }
    }

    /// Probe an existing control file without changing the census namespace.
    /// A missing control file cannot have an admitted publication owner.
    fn is_busy(path: &Path) -> Result<bool, MapCacheError> {
        let file = match OpenOptions::new().read(true).write(true).open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        match file.try_lock() {
            Ok(()) => {
                let _probe = Self { file };
                Ok(false)
            }
            Err(std::fs::TryLockError::WouldBlock) => Ok(true),
            Err(std::fs::TryLockError::Error(error)) => Err(error.into()),
        }
    }

    fn try_acquire_shared(path: &Path) -> Result<Option<Self>, MapCacheError> {
        let file = Self::open_control(path)?;
        match file.try_lock_shared() {
            Ok(()) => Ok(Some(Self { file })),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(error)) => Err(error.into()),
        }
    }
}

impl Drop for PublishLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

/// The existing capacity lock file carries a checked generation so a bounded
/// directory census can detect product mutations between cooperative units.
/// This is a live, lock-ordered change counter, not crash-durable artifact
/// metadata. New processes census the filesystem before using it.
struct CapacityLock {
    guard: PublishLock,
    generation: u64,
}

impl CapacityLock {
    fn try_acquire(path: &Path) -> Result<Option<Self>, MapCacheError> {
        let Some(guard) = PublishLock::try_acquire(path)? else {
            return Ok(None);
        };
        let mut file = &guard.file;
        let length = file.metadata()?.len();
        let generation = match length {
            0 => {
                file.seek(SeekFrom::Start(0))?;
                file.write_all(&0u64.to_le_bytes())?;
                0
            }
            CAPACITY_GENERATION_BYTES => {
                let mut bytes = [0; std::mem::size_of::<u64>()];
                file.seek(SeekFrom::Start(0))?;
                file.read_exact(&mut bytes)?;
                u64::from_le_bytes(bytes)
            }
            _ => {
                return Err(MapCacheError::Map(MapError::Invalid(
                    "map-cache capacity generation metadata",
                )))
            }
        };
        Ok(Some(Self { guard, generation }))
    }

    fn generation(&self) -> u64 {
        self.generation
    }

    fn bump(&mut self) -> Result<u64, MapCacheError> {
        let generation =
            self.generation
                .checked_add(1)
                .ok_or(MapCacheError::Map(MapError::Limit(
                    "map-cache capacity generation",
                )))?;
        let mut file = &self.guard.file;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&generation.to_le_bytes())?;
        self.generation = generation;
        Ok(generation)
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), MapCacheError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    atomic_write_file(path, bytes, true)
}

// Callers prepare and account for parent directories before writing the file.
fn atomic_write_file(path: &Path, bytes: &[u8], sync_parent: bool) -> Result<(), MapCacheError> {
    let parent = path
        .parent()
        .ok_or_else(|| MapCacheError::Message("map artifact has no parent".into()))?;
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
        run_once(|| fs::rename(&temp, path))?;
        Ok::<(), io::Error>(())
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(&temp);
        return Err(error.into());
    }
    if sync_parent {
        sync_directory(parent)?;
    }
    Ok(())
}

#[cfg(all(test, not(windows)))]
thread_local! {
    static TEST_DIRECTORY_SYNCS: std::cell::RefCell<Option<Vec<PathBuf>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(not(windows))]
fn sync_directory(path: &Path) -> Result<(), MapCacheError> {
    #[cfg(test)]
    TEST_DIRECTORY_SYNCS.with(|trace| {
        if let Some(paths) = trace.borrow_mut().as_mut() {
            paths.push(path.to_path_buf());
        }
    });
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

// WorkTicket's frozen marker contains a function pointer, which the seam checker refuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MapBakeTicket {
    identity: WorkIdentity,
    revision: InputRevision,
}

impl MapBakeTicket {
    fn new(ticket: WorkTicket<MapBakeSpec>) -> Self {
        Self {
            identity: ticket.identity(),
            revision: ticket.revision(),
        }
    }

    fn work_ticket(self) -> WorkTicket<MapBakeSpec> {
        WorkTicket::new(self.identity, self.revision)
    }
}

struct ActiveJob {
    key: JobKey,
    descriptor: MapProfileDescriptor,
    generation: u64,
    demand: Arc<AtomicUsize>,
    consumers: Arc<AtomicUsize>,
    status: Arc<Mutex<MapJobStatus>>,
    ready: Arc<Mutex<Option<ReadyMap>>>,
    artifact: ArtifactKind,
    ticket: Option<MapBakeTicket>,
    progress_sequence: u64,
    cancel_requested: bool,
}
struct PendingJob {
    active: ActiveJob,
    input: MapBakeInput,
    request: BakeRequest,
    capacity_sequence: Option<PublishSequence>,
}
struct ManagerJobs {
    active: Option<ActiveJob>,
    pending: Option<PendingJob>,
    next_generation: u64,
}
struct ReadyCache {
    catalogues: BTreeMap<Digest, Weak<ReadyCatalogue>>,
    images: BTreeMap<Digest, Weak<ReadyImages>>,
}

fn ready_slot_satisfies(
    slot: &Mutex<Option<ReadyMap>>,
    artifact: ArtifactKind,
    demand: MapDemand,
) -> bool {
    slot.lock().as_ref().is_some_and(|ready| match demand {
        MapDemand::CatalogueOnly => true,
        MapDemand::Images => ready.images.is_some(),
        MapDemand::ReadyImages => ready.images.is_some() || artifact == ArtifactKind::Images,
    })
}

struct ManagerInner {
    root: MapCacheRoot,
    producer: Arc<dyn MapBakeProducer>,
    shipped: Option<Arc<ShippedSource>>,
    jobs: Mutex<ManagerJobs>,
    ready: Mutex<ReadyCache>,
}

impl Drop for ManagerInner {
    fn drop(&mut self) {
        let Some(ticket) = self
            .jobs
            .get_mut()
            .active
            .as_ref()
            .and_then(|active| active.ticket)
        else {
            return;
        };
        // No manager remains to consume this job's eventual terminal. Abandon
        // custody now; WorkAdmission keeps its charge until the worker returns.
        if let Ok(shared) = WorkService::shared() {
            shared
                .lock()
                .abandon(ticket.work_ticket(), Cancellation::ScopeInvalidated);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MapBakeJobStage {
    PreparePaths,
    Census,
    Prune,
    OpenExisting,
    PrepareCursor,
    Cursor,
    Serialize,
    Finish,
    Publish,
    OpenPublished,
    Complete,
}

enum OpenReady {
    Busy,
    Stale,
    Ready(Option<ReadyMap>),
}

enum ExclusiveAcquire {
    Busy,
    Stale,
    Locked(MutationLock),
}

struct MutationLock {
    _capacity: CapacityLock,
    _key: PublishLock,
}

struct MapBakeJob {
    input: MapBakeInput,
    root: MapCacheRoot,
    producer: Arc<dyn MapBakeProducer>,
    shipped: Option<Arc<ShippedSource>>,
    descriptor: MapProfileDescriptor,
    request: BakeRequest,
    demand: Arc<AtomicUsize>,
    cancel: Arc<AtomicBool>,
    lock_path: PathBuf,
    ready_path: Arc<PathBuf>,
    stage: MapBakeJobStage,
    paths_prepared: bool,
    after_census: MapBakeJobStage,
    cursor: Option<Box<dyn MapBakeCursor>>,
    cursor_is_shipped: bool,
    shipped_checked: bool,
    shipped_bundle: Option<ShippedImages>,
    local_bake_started: bool,
    writer: Option<BakeWriter>,
    cursor_output: Option<MapBakeCursorOutput>,
    bake_output: Option<BakeOutput>,
    ready_result: Option<ReadyMap>,
    census_cursor: Option<CacheCensusCursor>,
    census_result: Option<CacheCensusResult>,
    prune_cursor: Option<PruneCursor>,
    recovery_passes: u32,
    roots_pruned: u32,
    last_cursor_progress: Option<MapProgressSnapshot>,
    progress_boundary: bool,
}

impl MapBakeJob {
    fn new(
        input: MapBakeInput,
        root: MapCacheRoot,
        producer: Arc<dyn MapBakeProducer>,
        shipped: Option<Arc<ShippedSource>>,
        request: BakeRequest,
        demand: Arc<AtomicUsize>,
    ) -> Result<Self, MapCacheError> {
        let descriptor = request.descriptor.clone();
        let key = match input.artifact {
            ArtifactKind::Catalogue => descriptor.catalogue_key()?,
            ArtifactKind::Images => descriptor.image_key()?,
        };
        if input.key.image != descriptor.image_key()?
            || input.key.catalogue != descriptor.catalogue_key()?
            || request.artifact != input.artifact
            || request.partial_dir.as_os_str().is_empty()
        {
            return Err(MapCacheError::Map(MapError::Identity));
        }
        let ready_path = Arc::new(match input.artifact {
            ArtifactKind::Catalogue => root.catalogue_dir(descriptor.catalogue_identity())?,
            ArtifactKind::Images => root.image_dir(descriptor.image_identity())?,
        });
        let lock_path = root.lock_path(descriptor.revision(), input.artifact, key);
        let census_cursor = CacheCensusCursor::new(root.clone());
        Ok(Self {
            input,
            root,
            producer,
            shipped,
            descriptor,
            request,
            demand,
            cancel: Arc::new(AtomicBool::new(false)),
            lock_path,
            ready_path,
            stage: MapBakeJobStage::PreparePaths,
            paths_prepared: false,
            after_census: MapBakeJobStage::OpenExisting,
            cursor: None,
            cursor_is_shipped: false,
            shipped_checked: false,
            shipped_bundle: None,
            local_bake_started: false,
            writer: None,
            cursor_output: None,
            bake_output: None,
            ready_result: None,
            census_cursor: Some(census_cursor),
            census_result: None,
            prune_cursor: None,
            recovery_passes: 0,
            roots_pruned: 0,
            last_cursor_progress: None,
            progress_boundary: false,
        })
    }

    fn open_ready(&self) -> Result<OpenReady, MapCacheError> {
        let census = self
            .census_result
            .as_ref()
            .ok_or(MapCacheError::Map(MapError::Path))?;
        let Some(mut capacity) = self.root.try_acquire_capacity()? else {
            return Ok(OpenReady::Busy);
        };
        if capacity.generation() != census.generation {
            return Ok(OpenReady::Stale);
        }
        let Some(catalogue) = self
            .root
            .open_catalogue(self.descriptor.catalogue_identity(), &mut capacity)?
        else {
            return Ok(OpenReady::Ready(None));
        };
        let images = if self.input.artifact == ArtifactKind::Images {
            self.root
                .open_images(self.descriptor.image_identity(), &mut capacity)?
        } else {
            None
        };
        Ok(OpenReady::Ready(Some(ReadyMap { catalogue, images })))
    }

    fn prepare_paths(&mut self) -> Result<bool, MapCacheError> {
        if self.input.artifact == ArtifactKind::Images
            && self.demand.load(AtomicOrdering::Acquire) != DEMAND_BAKE_IMAGES
            && !real_directory(&self.ready_path)?
        {
            self.ensure_shipped_bundle();
            if self.shipped_bundle.is_none() {
                return Ok(true);
            }
        }
        let Some(mut capacity) = self.root.try_acquire_capacity()? else {
            return Ok(false);
        };
        let identities = [
            Some(ArtifactIdentity::Catalogue(
                self.descriptor.catalogue_identity(),
            )),
            (self.input.artifact == ArtifactKind::Images)
                .then(|| ArtifactIdentity::Image(self.descriptor.image_identity())),
        ];
        let mut changed = false;
        for identity in identities.into_iter().flatten() {
            let (kind, key) = match identity {
                ArtifactIdentity::Catalogue(identity) => {
                    (ArtifactKind::Catalogue, identity.key()?.0)
                }
                ArtifactIdentity::Image(identity) => (ArtifactKind::Images, identity.key()?.0),
            };
            changed |= self
                .root
                .ensure_kind_dir(self.descriptor.revision(), kind)?;
            changed |= self
                .root
                .ensure_key_lock_file(self.descriptor.revision(), kind, key)?;
        }
        if changed {
            capacity.bump()?;
        }
        self.paths_prepared = true;
        Ok(true)
    }

    fn start_census(&mut self) {
        if !matches!(
            self.stage,
            MapBakeJobStage::PreparePaths | MapBakeJobStage::Census | MapBakeJobStage::Prune
        ) {
            self.after_census = self.stage;
        }
        self.census_result = None;
        self.census_cursor = None;
        self.census_cursor = Some(CacheCensusCursor::new(self.root.clone()));
        self.stage = MapBakeJobStage::Census;
    }

    fn start_prune(
        &mut self,
        census: &CacheCensusResult,
        root_index: usize,
    ) -> Result<(), MapCacheError> {
        self.prune_cursor = Some(PruneCursor::new(
            self.root.clone(),
            &census.roots[root_index],
            census.generation,
        )?);
        self.recovery_passes = self
            .recovery_passes
            .checked_add(1)
            .ok_or(MapCacheError::Map(MapError::Limit(
                "map-cache recovery passes",
            )))?;
        self.census_result = None;
        self.census_cursor = None;
        self.stage = MapBakeJobStage::Prune;
        Ok(())
    }

    fn accept_census(&mut self, census: CacheCensusResult) -> Result<(), MapCacheError> {
        if census.foreign_tree {
            return Err(MapCacheError::Map(MapError::Path));
        }
        if let Some(index) = census.roots.iter().position(|root| root.marked_for_prune) {
            return self.start_prune(&census, index);
        }
        if census.overrun.is_some() || census.bytes > MAX_GENERATED_CACHE_BYTES {
            let candidate = census
                .roots
                .iter()
                .enumerate()
                .filter(|(_, root)| {
                    root.fully_censused
                        && root.safe_to_prune
                        && !root.publication_busy
                        && root.identity.is_some()
                        && root.entries <= MAX_ARTIFACT_ROOT_ENTRIES
                })
                .min_by(|(_, a), (_, b)| {
                    a.lru.cmp(&b.lru).then_with(|| compare_root_id(a.id, b.id))
                })
                .map(|(index, _)| index);
            if let Some(index) = candidate {
                return self.start_prune(&census, index);
            }
            return Err(MapCacheError::Map(MapError::Limit(
                if census.overrun.is_some() {
                    "map-cache census bounds"
                } else {
                    "generated map cache bytes"
                },
            )));
        }
        self.validate_census_targets(&census)?;
        if self.recovery_passes != 0 {
            api::host_log!(
                api::hostlog::Category::Lifecycle,
                api::hostlog::Level::Info,
                "map cache recovery passes={} roots_pruned={} entries={} bytes={}",
                self.recovery_passes,
                self.roots_pruned,
                census.entries,
                census.bytes
            );
            self.recovery_passes = 0;
            self.roots_pruned = 0;
        }
        self.census_cursor = None;
        self.census_result = Some(census);
        let usage = self.writer_usage()?;
        if usage.artifact_entries == 0
            && (self.writer.is_some()
                || matches!(
                    self.after_census,
                    MapBakeJobStage::Serialize | MapBakeJobStage::Finish | MapBakeJobStage::Publish
                ))
        {
            // A completed concurrent prune removed this unpublished partial.
            // Rebuild from its now-empty durable state, never from stale tile
            // receipts retained by the old cursor.
            self.writer = None;
            self.cursor = None;
            self.cursor_output = None;
            self.bake_output = None;
            self.cursor_is_shipped = false;
            self.shipped_checked = false;
            self.shipped_bundle = None;
            self.after_census = MapBakeJobStage::OpenExisting;
        } else if let Some(writer) = &mut self.writer {
            // Other keys can change while this job yields. Its next write must
            // use the refreshed whole-cache totals, not its original census.
            writer.cache_bytes = usage.bytes;
            writer.cache_entries = usage.entries;
            writer.cache_product_entries = usage.product_entries;
            writer.artifact_entries = usage.artifact_entries;
        }
        self.stage = self.after_census;
        Ok(())
    }

    fn validate_census_targets(&self, census: &CacheCensusResult) -> Result<(), MapCacheError> {
        let mut identities = [None, None];
        identities[0] = Some(ArtifactIdentity::Catalogue(
            self.descriptor.catalogue_identity(),
        ));
        if self.input.artifact == ArtifactKind::Images {
            identities[1] = Some(ArtifactIdentity::Image(self.descriptor.image_identity()));
        }
        for identity in identities.into_iter().flatten() {
            let id = ArtifactRootId {
                revision: self.descriptor.revision(),
                kind: match identity {
                    ArtifactIdentity::Catalogue(_) => ArtifactKind::Catalogue,
                    ArtifactIdentity::Image(_) => ArtifactKind::Images,
                },
                key: identity.key()?,
                partial: false,
            };
            for partial in [false, true] {
                let root_id = ArtifactRootId { partial, ..id };
                if let Some(root) = census
                    .roots
                    .iter()
                    .find(|root| root.id == root_id && root.root_exists)
                {
                    if !root.fully_censused
                        || (root.identity.is_some() && root.identity != Some(identity))
                        || (!partial && (!root.safe_to_prune || root.identity != Some(identity)))
                    {
                        return Err(MapCacheError::Map(MapError::Path));
                    }
                }
            }
        }
        Ok(())
    }

    fn ensure_shipped_bundle(&mut self) {
        if self.shipped_checked {
            return;
        }
        self.shipped_checked = true;
        self.shipped_bundle = self
            .shipped
            .as_ref()
            .and_then(|source| source.bundle(self.descriptor.image_identity()));
    }

    fn ready_satisfies(&mut self, ready: &ReadyMap) -> bool {
        match self.input.artifact {
            ArtifactKind::Catalogue => true,
            ArtifactKind::Images if ready.images.is_some() => true,
            ArtifactKind::Images
                if self.demand.load(AtomicOrdering::Acquire) == DEMAND_BAKE_IMAGES =>
            {
                false
            }
            ArtifactKind::Images => {
                self.ensure_shipped_bundle();
                self.shipped_bundle.is_none()
            }
        }
    }

    fn begin_cursor(&mut self) -> Result<bool, MapCacheError> {
        if self.cursor.is_some() {
            return Ok(true);
        }
        let partial_dir = Arc::clone(&self.request.partial_dir);
        if self.input.artifact == ArtifactKind::Images && !self.local_bake_started {
            self.ensure_shipped_bundle();
            if let Some(bundle) = self.shipped_bundle.take() {
                let Some(source) = &self.shipped else {
                    return Err(MapCacheError::Map(MapError::Invalid(
                        "shipped source absent",
                    )));
                };
                self.cursor = Some(source.cursor(
                    self.descriptor.image_identity(),
                    bundle,
                    Arc::clone(&partial_dir),
                )?);
                self.cursor_is_shipped = true;
                return Ok(true);
            }
            if self.demand.load(AtomicOrdering::Acquire) != DEMAND_BAKE_IMAGES {
                return Ok(false);
            }
            self.local_bake_started = true;
        }
        self.cursor = Some(self.producer.cursor(&self.request, partial_dir)?);
        self.cursor_is_shipped = false;
        Ok(true)
    }

    fn writer_usage(&self) -> Result<CacheUsage, MapCacheError> {
        let census = self
            .census_result
            .as_ref()
            .ok_or(MapCacheError::Map(MapError::Path))?;
        let identity = self.request.expected_identity();
        let id = ArtifactRootId {
            revision: self.descriptor.revision(),
            kind: self.input.artifact,
            key: identity.key()?,
            partial: true,
        };
        Ok(CacheUsage::for_root(census, id))
    }

    fn open_writer(&mut self, plan: BakePlan) -> Result<(), MapCacheError> {
        let progress: Arc<dyn Fn(MapProgress) + Send + Sync> = Arc::new(|_| {});
        let mut writer = BakeWriter::open_at(
            &self.root,
            Arc::clone(&self.request.partial_dir),
            &self.request,
            plan,
            Arc::clone(&self.cancel),
            progress,
            self.writer_usage()?,
        )?;
        if self.input.artifact == ArtifactKind::Images {
            writer.set_stage(plan.stage, "preparing map artifact")?;
        }
        self.writer = Some(writer);
        Ok(())
    }

    fn acquire_exclusive(&mut self) -> Result<ExclusiveAcquire, MapCacheError> {
        let census = self
            .census_result
            .as_ref()
            .ok_or(MapCacheError::Map(MapError::Path))?;
        let Some(mut capacity) = self.root.try_acquire_capacity()? else {
            return Ok(ExclusiveAcquire::Busy);
        };
        if capacity.generation() != census.generation {
            return Ok(ExclusiveAcquire::Stale);
        }
        let Some(key) = PublishLock::try_acquire(&self.lock_path)? else {
            return Ok(ExclusiveAcquire::Busy);
        };
        let expected = self.request.expected_identity();
        let root_id = ArtifactRootId {
            revision: self.descriptor.revision(),
            kind: self.input.artifact,
            key: expected.key()?,
            partial: false,
        };
        let marker_path = self.root.prune_marker_path(root_id);
        if let Some((_, marker_identity)) =
            read_prune_marker(&marker_path, root_id.revision, root_id.kind, root_id.key)?
        {
            if marker_identity != expected || !identity_matches_root(marker_identity, root_id) {
                return Err(MapCacheError::Map(MapError::Path));
            }
            return Ok(ExclusiveAcquire::Busy);
        }
        let generation = capacity.bump()?;
        self.census_result
            .as_mut()
            .ok_or(MapCacheError::Map(MapError::Path))?
            .generation = generation;
        Ok(ExclusiveAcquire::Locked(MutationLock {
            _capacity: capacity,
            _key: key,
        }))
    }
    fn record_completed_units(&mut self, units: Vec<CompletedUnit>) -> Result<(), MapCacheError> {
        if units.is_empty() {
            return Ok(());
        }
        let writer = self
            .writer
            .as_mut()
            .ok_or(MapCacheError::Map(MapError::Invalid(
                "producer completed units before plan",
            )))?;
        // New writes are already recorded; sorting lets the writer adopt
        // uncheckpointed resume files in canonical unit order.
        let mut units = units;
        units.sort_unstable_by_key(|unit| unit.key);
        for unit in units {
            writer.record_unit(unit.key, unit.payload)?;
        }
        Ok(())
    }

    fn publish_cursor_progress(
        &mut self,
        context: &mut StepContext<'_, MapBakePhase>,
        progress: MapProgressSnapshot,
        final_progress: bool,
    ) {
        let progress = normalize_cursor_progress(progress);
        let previous = self.last_cursor_progress;
        if !should_publish_cursor_progress(previous, progress, final_progress) {
            return;
        }
        context.publish_progress(
            map_bake_phase(progress.stage),
            progress.completed,
            progress.total,
        );
        self.last_cursor_progress = Some(progress);
        self.progress_boundary |= cursor_progress_boundary(previous, progress, final_progress);
    }

    fn persist_checkpoint(&mut self) -> Result<(), MapCacheError> {
        if let Some(writer) = &mut self.writer {
            // A catalogue has one all-or-nothing payload. Before that
            // payload exists, an empty checkpoint preserves no work.
            if writer.artifact() == ArtifactKind::Catalogue
                && writer.checkpoint.completed.as_slice().is_empty()
            {
                return Ok(());
            }
            writer.persist_checkpoint()?;
        }
        Ok(())
    }

    fn fail(&mut self, error: MapCacheError) -> StepOutcome<MapBakeResult, MapCacheError> {
        StepOutcome::Failed(error)
    }

    fn cancel(&mut self) -> StepOutcome<MapBakeResult, MapCacheError> {
        self.cancel.store(true, AtomicOrdering::Release);
        StepOutcome::Yielded
    }

    fn finish_cancel(&mut self) -> StepOutcome<MapBakeResult, MapCacheError> {
        if self
            .writer
            .as_ref()
            .is_some_and(|writer| writer.checkpoint_dirty)
        {
            match self.acquire_exclusive() {
                Ok(ExclusiveAcquire::Locked(_lock)) => {
                    if let Err(error) = self.persist_checkpoint() {
                        return self.fail(error);
                    }
                }
                // Without custody, retain the last durable batch for resume.
                Ok(ExclusiveAcquire::Busy | ExclusiveAcquire::Stale) => {}
                Err(error) => return self.fail(error),
            }
        }
        self.cancel()
    }

    fn consume_host_unit(
        &mut self,
        context: &mut StepContext<'_, MapBakePhase>,
    ) -> StepOutcome<MapBakeResult, MapCacheError> {
        let checkpoint = context.consume_unit();
        #[cfg(test)]
        MAP_BAKE_TEST_UNITS.fetch_add(1, AtomicOrdering::Relaxed);
        match checkpoint {
            WorkCheckpoint::Continue | WorkCheckpoint::Yield => StepOutcome::Yielded,
            WorkCheckpoint::Cancel(_) => self.cancel(),
        }
    }

    fn serialize_output(&mut self) -> Result<(), MapCacheError> {
        let output = self
            .cursor_output
            .take()
            .ok_or(MapCacheError::Map(MapError::Invalid(
                "map producer result absent",
            )))?;
        match output {
            MapBakeCursorOutput::Catalogue(pois) => {
                let identity = self.descriptor.catalogue_identity();
                if pois.identity != identity {
                    return Err(MapCacheError::Map(MapError::Identity));
                }
                let payload = pois.encode()?;
                let payload_bytes = u32::try_from(payload.len())
                    .map_err(|_| MapCacheError::Map(MapError::Limit("catalogue payload")))?;
                self.writer
                    .as_mut()
                    .ok_or(MapCacheError::Map(MapError::Invalid(
                        "catalogue writer absent",
                    )))?
                    .publish_unit(UnitKey::ClientPois, &payload)?;
                self.bake_output = Some(BakeOutput::Catalogue(CatalogueManifest {
                    schema: CATALOGUE_SCHEMA,
                    identity,
                    key: identity.key()?,
                    record_count: u32::try_from(pois.records.as_slice().len())
                        .map_err(|_| MapCacheError::Map(MapError::Limit("POI count")))?,
                    payload: PayloadReceipt {
                        bytes: payload_bytes,
                        sha256: Digest::of(&payload),
                    },
                }));
            }
            MapBakeCursorOutput::Images(manifest) => {
                if manifest.identity != self.descriptor.image_identity() {
                    return Err(MapCacheError::Map(MapError::Identity));
                }
                self.bake_output = Some(BakeOutput::Images(manifest));
            }
        }
        Ok(())
    }

    fn work_result(&self, ready: ReadyMap) -> MapBakeResult {
        MapBakeResult {
            key: self.input.key,
            generation: self.input.generation,
            artifact: self.input.artifact,
            ready,
        }
    }

    fn take_work_result(&mut self) -> Result<MapBakeResult, MapCacheError> {
        self.ready_result
            .take()
            .map(|ready| self.work_result(ready))
            .ok_or(MapCacheError::Map(MapError::Invalid("ready result absent")))
    }
    fn step_once(
        &mut self,
        context: &mut StepContext<'_, MapBakePhase>,
        cursor_lock: &mut Option<MutationLock>,
    ) -> StepOutcome<MapBakeResult, MapCacheError> {
        match context.checkpoint() {
            WorkCheckpoint::Continue => {}
            WorkCheckpoint::Yield => return StepOutcome::Yielded,
            WorkCheckpoint::Cancel(_) => return self.cancel(),
        }

        if !matches!(self.stage, MapBakeJobStage::Cursor) {
            *cursor_lock = None;
        }

        match self.stage {
            MapBakeJobStage::PreparePaths => match self.prepare_paths() {
                Ok(true) => {
                    self.stage = MapBakeJobStage::Census;
                    self.consume_host_unit(context)
                }
                Ok(false) => self.consume_host_unit(context),
                Err(error) => self.fail(error),
            },
            MapBakeJobStage::Census => {
                let result = match self.census_cursor.as_mut() {
                    Some(cursor) => cursor.advance(),
                    None => return self.fail(MapCacheError::Map(MapError::Path)),
                };
                match result {
                    Ok(Some(census)) => match self.accept_census(census) {
                        Ok(()) => self.consume_host_unit(context),
                        Err(error) => self.fail(error),
                    },
                    Ok(None) => self.consume_host_unit(context),
                    Err(error) => self.fail(error),
                }
            }
            MapBakeJobStage::Prune => {
                let result = match self.prune_cursor.as_mut() {
                    Some(cursor) => cursor.advance(),
                    None => return self.fail(MapCacheError::Map(MapError::Path)),
                };
                match result {
                    Ok(PruneAdvance::Pending) => self.consume_host_unit(context),
                    Ok(PruneAdvance::Stale) => {
                        self.prune_cursor = None;
                        self.start_census();
                        self.consume_host_unit(context)
                    }
                    Ok(PruneAdvance::Complete) => {
                        self.prune_cursor = None;
                        self.roots_pruned = match self.roots_pruned.checked_add(1) {
                            Some(roots) => roots,
                            None => {
                                return self.fail(MapCacheError::Map(MapError::Limit(
                                    "map-cache pruned roots",
                                )))
                            }
                        };
                        self.start_census();
                        self.consume_host_unit(context)
                    }
                    Err(error) => self.fail(error),
                }
            }
            MapBakeJobStage::OpenExisting => match self.open_ready() {
                Ok(OpenReady::Busy) => self.consume_host_unit(context),
                Ok(OpenReady::Ready(Some(ready))) if self.ready_satisfies(&ready) => {
                    self.ready_result = Some(ready);
                    self.stage = MapBakeJobStage::Complete;
                    self.consume_host_unit(context)
                }
                Ok(OpenReady::Ready(_)) => {
                    self.stage = MapBakeJobStage::PrepareCursor;
                    self.consume_host_unit(context)
                }
                Ok(OpenReady::Stale) => {
                    self.start_census();
                    self.consume_host_unit(context)
                }
                Err(error) => self.fail(error),
            },
            MapBakeJobStage::PrepareCursor => {
                if !self.paths_prepared {
                    if self.demand.load(AtomicOrdering::Acquire) == DEMAND_BAKE_IMAGES {
                        self.start_census();
                        self.stage = MapBakeJobStage::PreparePaths;
                    } else {
                        self.stage = MapBakeJobStage::OpenPublished;
                    }
                    return self.consume_host_unit(context);
                }
                let lock = match self.acquire_exclusive() {
                    Ok(ExclusiveAcquire::Locked(lock)) => lock,
                    Ok(ExclusiveAcquire::Busy) => return self.consume_host_unit(context),
                    Ok(ExclusiveAcquire::Stale) => {
                        self.start_census();
                        return self.consume_host_unit(context);
                    }
                    Err(error) => return self.fail(error),
                };
                match real_directory(&self.ready_path) {
                    Ok(true) => {
                        drop(lock);
                        self.stage = MapBakeJobStage::OpenPublished;
                        return self.consume_host_unit(context);
                    }
                    Ok(false) => {}
                    Err(error) => return self.fail(error),
                }
                match self.begin_cursor() {
                    Ok(false) => {
                        drop(lock);
                        self.stage = MapBakeJobStage::OpenPublished;
                        self.consume_host_unit(context)
                    }
                    Err(error) => self.fail(error),
                    Ok(true) => {
                        let plan = self.cursor.as_ref().and_then(|cursor| cursor.plan());
                        if let Some(plan) = plan {
                            if let Some(writer) = &self.writer {
                                if writer.checkpoint().planned_units != plan.planned_units {
                                    return self.fail(MapCacheError::Map(MapError::Invalid(
                                        "shipped and local image plans differ",
                                    )));
                                }
                            } else if let Err(error) = self.open_writer(plan) {
                                return self.fail(error);
                            }
                        }
                        self.stage = MapBakeJobStage::Cursor;
                        self.consume_host_unit(context)
                    }
                }
            }
            MapBakeJobStage::Cursor => {
                // Input-only preflight needs no cache ownership. Once output
                // is possible, reuse ownership throughout this worker batch.
                if cursor_lock.is_none()
                    && (self.writer.is_some()
                        || self
                            .cursor
                            .as_ref()
                            .and_then(|cursor| cursor.plan())
                            .is_some())
                {
                    let lock = match self.acquire_exclusive() {
                        Ok(ExclusiveAcquire::Locked(lock)) => lock,
                        Ok(ExclusiveAcquire::Busy) => return self.consume_host_unit(context),
                        Ok(ExclusiveAcquire::Stale) => {
                            self.start_census();
                            return self.consume_host_unit(context);
                        }
                        Err(error) => return self.fail(error),
                    };
                    match real_directory(&self.ready_path) {
                        Ok(true) => {
                            drop(lock);
                            self.stage = MapBakeJobStage::OpenPublished;
                            return self.consume_host_unit(context);
                        }
                        Ok(false) => {}
                        Err(error) => return self.fail(error),
                    }
                    *cursor_lock = Some(lock);
                }
                if self.writer.is_none() {
                    if let Some(plan) = self.cursor.as_ref().and_then(|cursor| cursor.plan()) {
                        if let Err(error) = self.open_writer(plan) {
                            return self.fail(error);
                        }
                        return self.consume_host_unit(context);
                    }
                }
                if let (Some(plan), Some(writer)) = (
                    self.cursor.as_ref().and_then(|cursor| cursor.plan()),
                    self.writer.as_ref(),
                ) {
                    if writer.checkpoint().planned_units != plan.planned_units {
                        return self.fail(MapCacheError::Map(MapError::Invalid(
                            "shipped and local image plans differ",
                        )));
                    }
                }
                if let Some(writer) = self.writer.as_mut() {
                    match writer.verify_next_checkpoint_unit() {
                        Ok(true) => return self.consume_host_unit(context),
                        Ok(false) => {}
                        Err(error) => return self.fail(error),
                    }
                }
                let (result, completed, progress, plan) = {
                    let writer = self.writer.as_mut();
                    let cursor = match self.cursor.as_mut() {
                        Some(cursor) => cursor,
                        None => {
                            return self
                                .fail(MapCacheError::Map(MapError::Invalid("map cursor absent")))
                        }
                    };
                    #[cfg(test)]
                    let units_before = context.units_this_slice();
                    let result = cursor.step(context, writer);
                    #[cfg(test)]
                    MAP_BAKE_TEST_UNITS.fetch_add(
                        u64::from(context.units_this_slice().saturating_sub(units_before)),
                        AtomicOrdering::Relaxed,
                    );
                    (
                        result,
                        cursor.take_completed_units(),
                        cursor.progress(),
                        cursor.plan(),
                    )
                };
                if let Err(error) = self.record_completed_units(completed) {
                    return self.fail(error);
                }
                let final_progress = matches!(&result, Ok(MapBakeCursorStep::Completed(_)));
                self.publish_cursor_progress(context, progress, final_progress);
                match result {
                    Err(MapCacheError::Cancelled) => {
                        if let Err(error) = self.persist_checkpoint() {
                            return self.fail(error);
                        }
                        self.fail(MapCacheError::Cancelled)
                    }
                    Err(error) => self.fail(error),
                    Ok(MapBakeCursorStep::Cancelled) => self.cancel(),
                    Ok(MapBakeCursorStep::Rejected(error)) if self.cursor_is_shipped => {
                        // Preserve the verified shipped prefix before either
                        // a consented native fallback or a later demand.
                        if let Err(error) = self.persist_checkpoint() {
                            return self.fail(error);
                        }
                        if let Some(source) = &self.shipped {
                            if let Ok(key) = self.descriptor.image_key() {
                                source.reject(key, &error);
                            }
                        }
                        self.shipped_bundle = None;
                        self.cursor = None;
                        self.cursor_is_shipped = false;
                        if self.demand.load(AtomicOrdering::Acquire) == DEMAND_BAKE_IMAGES {
                            self.stage = MapBakeJobStage::PrepareCursor;
                        } else {
                            self.stage = MapBakeJobStage::OpenPublished;
                        }
                        StepOutcome::Yielded
                    }
                    Ok(MapBakeCursorStep::Rejected(error)) => self.fail(error),
                    Ok(MapBakeCursorStep::Yielded) => {
                        if let Some(plan) = plan {
                            if self.writer.as_ref().is_some_and(|writer| {
                                writer.checkpoint().planned_units != plan.planned_units
                            }) {
                                return self.fail(MapCacheError::Map(MapError::Invalid(
                                    "shipped and local image plans differ",
                                )));
                            }
                        }
                        match context.checkpoint() {
                            WorkCheckpoint::Continue => {}
                            WorkCheckpoint::Yield => return StepOutcome::Yielded,
                            WorkCheckpoint::Cancel(_) => return self.cancel(),
                        }
                        StepOutcome::Yielded
                    }
                    Ok(MapBakeCursorStep::Completed(output)) => {
                        self.cursor_output = Some(output);
                        self.stage = MapBakeJobStage::Serialize;
                        if let Err(error) = self.persist_checkpoint() {
                            return self.fail(error);
                        }
                        StepOutcome::Yielded
                    }
                }
            }
            MapBakeJobStage::Serialize => {
                // The completed cursor result is owned separately; release its
                // scratch before host serialization allocates bounded JSON buffers.
                self.cursor = None;
                self.cursor_is_shipped = false;
                let lock = match self.acquire_exclusive() {
                    Ok(ExclusiveAcquire::Locked(lock)) => lock,
                    Ok(ExclusiveAcquire::Busy) => return self.consume_host_unit(context),
                    Ok(ExclusiveAcquire::Stale) => {
                        self.start_census();
                        return self.consume_host_unit(context);
                    }
                    Err(error) => return self.fail(error),
                };
                if let Err(error) = self.serialize_output() {
                    return self.fail(error);
                }
                self.stage = MapBakeJobStage::Finish;
                drop(lock);
                self.consume_host_unit(context)
            }
            MapBakeJobStage::Finish => {
                let lock = match self.acquire_exclusive() {
                    Ok(ExclusiveAcquire::Locked(lock)) => lock,
                    Ok(ExclusiveAcquire::Busy) => return self.consume_host_unit(context),
                    Ok(ExclusiveAcquire::Stale) => {
                        self.start_census();
                        return self.consume_host_unit(context);
                    }
                    Err(error) => return self.fail(error),
                };
                let output = match self.bake_output.take() {
                    Some(output) => output,
                    None => {
                        return self.fail(MapCacheError::Map(MapError::Invalid(
                            "map artifact manifest absent",
                        )))
                    }
                };
                let writer = match self.writer.take() {
                    Some(writer) => writer,
                    None => {
                        return self.fail(MapCacheError::Map(MapError::Invalid(
                            "map artifact writer absent",
                        )))
                    }
                };
                match writer.finish(output) {
                    Ok(_) => {}
                    Err(error) => return self.fail(error),
                }
                self.stage = MapBakeJobStage::Publish;
                drop(lock);
                self.consume_host_unit(context)
            }
            MapBakeJobStage::Publish => {
                let lock = match self.acquire_exclusive() {
                    Ok(ExclusiveAcquire::Locked(lock)) => lock,
                    Ok(ExclusiveAcquire::Busy) => return self.consume_host_unit(context),
                    Ok(ExclusiveAcquire::Stale) => {
                        self.start_census();
                        return self.consume_host_unit(context);
                    }
                    Err(error) => return self.fail(error),
                };
                match publish_partial(&self.request.partial_dir, &self.ready_path) {
                    Ok(()) => self.stage = MapBakeJobStage::OpenPublished,
                    Err(error) => return self.fail(error),
                }
                drop(lock);
                self.consume_host_unit(context)
            }
            MapBakeJobStage::OpenPublished => match self.open_ready() {
                Ok(OpenReady::Busy) => self.consume_host_unit(context),
                Ok(OpenReady::Ready(Some(ready))) if self.ready_satisfies(&ready) => {
                    self.ready_result = Some(ready);
                    self.stage = MapBakeJobStage::Complete;
                    self.consume_host_unit(context)
                }
                Ok(OpenReady::Ready(Some(_)))
                    if self.input.artifact == ArtifactKind::Images
                        && self.demand.load(AtomicOrdering::Acquire) == DEMAND_BAKE_IMAGES
                        && !self.local_bake_started =>
                {
                    self.stage = MapBakeJobStage::PrepareCursor;
                    self.consume_host_unit(context)
                }
                Ok(OpenReady::Ready(_)) => self.fail(MapCacheError::Busy),
                Ok(OpenReady::Stale) => {
                    self.start_census();
                    self.consume_host_unit(context)
                }
                Err(error) => self.fail(error),
            },
            MapBakeJobStage::Complete => match context.checkpoint() {
                WorkCheckpoint::Continue => match self.take_work_result() {
                    Ok(result) => StepOutcome::Completed(result),
                    Err(error) => self.fail(error),
                },
                WorkCheckpoint::Yield => StepOutcome::Yielded,
                WorkCheckpoint::Cancel(_) => self.cancel(),
            },
        }
    }
}

impl CooperativeJob for MapBakeJob {
    type Output = MapBakeResult;
    type Error = MapCacheError;
    type Phase = MapBakePhase;

    fn request_thread(&self) -> Option<api::work::RequestThread<'_>> {
        Some(api::work::RequestThread::new("274bot-map-bake"))
    }

    fn step(
        &mut self,
        context: &mut StepContext<'_, Self::Phase>,
    ) -> StepOutcome<Self::Output, Self::Error> {
        #[cfg(test)]
        let _step_timer = MapBakeTestStepTimer::start();
        // This guard is local to one invocation. It cannot survive a yield.
        let mut cursor_lock = None;
        loop {
            let outcome = self.step_once(context, &mut cursor_lock);
            match outcome {
                StepOutcome::Yielded => {
                    let progress_boundary = std::mem::take(&mut self.progress_boundary);
                    let needs_cursor_lock = matches!(self.stage, MapBakeJobStage::Cursor)
                        && cursor_lock.is_none()
                        && (self.writer.is_some()
                            || self
                                .cursor
                                .as_ref()
                                .and_then(|cursor| cursor.plan())
                                .is_some());
                    match context.checkpoint() {
                        WorkCheckpoint::Continue if self.cancel.load(AtomicOrdering::Acquire) => {
                            drop(cursor_lock);
                            return self.finish_cancel();
                        }
                        WorkCheckpoint::Continue if progress_boundary || needs_cursor_lock => {
                            return StepOutcome::Yielded;
                        }
                        WorkCheckpoint::Continue => {}
                        WorkCheckpoint::Yield => return StepOutcome::Yielded,
                        WorkCheckpoint::Cancel(_) => {
                            drop(cursor_lock);
                            return self.finish_cancel();
                        }
                    }
                }
                outcome => return outcome,
            }
        }
    }

    fn retained(&self) -> RetainedState {
        RetainedState::clean(self.input.charge.total_bytes().unwrap_or(usize::MAX))
    }
}

fn map_bake_phase(stage: MapStage) -> MapBakePhase {
    match stage {
        MapStage::ReadingCache => MapBakePhase::ReadingCache,
        MapStage::DerivingPois => MapBakePhase::DerivingPois,
        MapStage::BakingPlane => MapBakePhase::BakingPlane,
        MapStage::BuildingZoomLevels => MapBakePhase::BuildingZoomLevels,
        MapStage::Publishing => MapBakePhase::Publishing,
    }
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
        Self::build(root, producer, None)
    }

    /// A manager that installs terrain shipped under `shipped` for an image
    /// identity that matches it exactly, instead of baking that terrain.
    pub fn with_shipped_images(
        root: MapCacheRoot,
        producer: Arc<dyn MapBakeProducer>,
        shipped: ShippedMapImages,
    ) -> Self {
        Self::build(root, producer, Some(Arc::new(ShippedSource::new(shipped))))
    }

    fn build(
        root: MapCacheRoot,
        producer: Arc<dyn MapBakeProducer>,
        shipped: Option<Arc<ShippedSource>>,
    ) -> Self {
        Self {
            inner: Arc::new(ManagerInner {
                root,
                producer,
                shipped,
                jobs: Mutex::new(ManagerJobs {
                    active: None,
                    pending: None,
                    next_generation: 1,
                }),
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

    /// Published terrain imagery for `descriptor` (baked earlier, or shipped
    /// and installed), independent of the catalogue: a missing or stale
    /// catalogue is derived cheaply by the worker, terrain is the expensive
    /// local bake. The returned value holds a shared lease, so capacity
    /// pruning cannot remove that terrain while it is retained.
    pub fn ready_images(
        &self,
        descriptor: &MapProfileDescriptor,
    ) -> Result<Option<Arc<ReadyImages>>, MapCacheError> {
        self.cached_images(descriptor.image_identity())
    }

    /// Terrain shipped with this release for exactly `descriptor`'s image
    /// identity, not yet found unusable in this process: the worker installs
    /// it instead of baking. Reads only the small shipped description; the
    /// worker verifies every file while installing.
    pub fn shipped_images_available(&self, descriptor: &MapProfileDescriptor) -> bool {
        self.inner
            .shipped
            .as_ref()
            .is_some_and(|shipped| shipped.bundle(descriptor.image_identity()).is_some())
    }

    fn input_for(
        &self,
        descriptor: &MapProfileDescriptor,
        key: JobKey,
        generation: u64,
        artifact: ArtifactKind,
    ) -> Result<(MapBakeInput, BakeRequest), MapCacheError> {
        let request = bake_request(&self.inner.root, descriptor.clone(), artifact)?;
        let input = MapBakeInput {
            key,
            generation,
            artifact,
            charge: host_map_bake_charge(&request, self.inner.producer.charge(&request)?)?,
        };
        Ok((input, request))
    }

    fn pending_for(
        &self,
        descriptor: MapProfileDescriptor,
        key: JobKey,
        generation: u64,
        demand: MapDemand,
        artifact: ArtifactKind,
    ) -> Result<PendingJob, MapCacheError> {
        let (input, request) = self.input_for(&descriptor, key, generation, artifact)?;
        let active = ActiveJob {
            key,
            descriptor,
            generation,
            demand: Arc::new(AtomicUsize::new(demand.flag())),
            consumers: Arc::new(AtomicUsize::new(1)),
            status: Arc::new(Mutex::new(MapJobStatus::Queued)),
            ready: Arc::new(Mutex::new(None)),
            artifact,
            ticket: None,
            progress_sequence: 0,
            cancel_requested: false,
        };
        Ok(PendingJob {
            active,
            input,
            request,
            capacity_sequence: None,
        })
    }

    fn running_handle(
        &self,
        descriptor: MapProfileDescriptor,
        demand: MapDemand,
        active: &ActiveJob,
    ) -> MapDemandHandle {
        MapDemandHandle::running(
            self.clone(),
            descriptor,
            demand,
            active.generation,
            Arc::clone(&active.status),
            Arc::clone(&active.ready),
        )
    }

    pub fn request(
        &self,
        descriptor: MapProfileDescriptor,
        demand: MapDemand,
    ) -> Result<MapDemandHandle, MapCacheError> {
        self.reap();
        let key = JobKey {
            image: descriptor.image_key()?,
            catalogue: descriptor.catalogue_key()?,
        };
        let handle_descriptor = descriptor.clone().without_input();
        // Cache hits still enter the admitted job's bounded preflight.
        let artifact = ArtifactKind::Catalogue;

        let mut superseded = None;
        let mut jobs = self.inner.jobs.lock();
        if let Some(pending) = jobs
            .pending
            .as_mut()
            .filter(|pending| pending.active.key == key)
        {
            upgrade_demand(&pending.active.demand, demand);
            pending
                .active
                .consumers
                .fetch_add(1, AtomicOrdering::AcqRel);
            let handle = self.running_handle(handle_descriptor, demand, &pending.active);
            drop(jobs);
            self.try_submit_pending(true);
            return Ok(handle);
        }
        if let Some(active) = jobs.active.as_mut().filter(|active| active.key == key) {
            let status = active.status.lock().clone();
            if matches!(
                status,
                MapJobStatus::Failed(_) | MapJobStatus::Paused | MapJobStatus::Cancelled
            ) {
                return Ok(MapDemandHandle::terminal(
                    self.clone(),
                    handle_descriptor,
                    demand,
                    status,
                ));
            }
            if !active.cancel_requested && !is_terminal(&status) {
                upgrade_demand(&active.demand, demand);
                active.consumers.fetch_add(1, AtomicOrdering::AcqRel);
                let handle = self.running_handle(handle_descriptor, demand, active);
                drop(jobs);
                self.try_submit_pending(true);
                return Ok(handle);
            }
            if status == MapJobStatus::Ready
                && ready_slot_satisfies(&active.ready, active.artifact, demand)
            {
                active.consumers.fetch_add(1, AtomicOrdering::AcqRel);
                return Ok(self.running_handle(handle_descriptor, demand, active));
            }
        }

        let active_is_live = jobs
            .active
            .as_ref()
            .is_some_and(|active| !is_terminal(&active.status.lock()));
        if active_is_live {
            if let Some(active) = jobs.active.as_mut() {
                if !active.cancel_requested {
                    active.cancel_requested = true;
                    superseded = active.ticket;
                }
            }
        } else {
            jobs.active.take();
        }

        let generation = jobs.next_generation;
        jobs.next_generation = generation
            .checked_add(1)
            .ok_or_else(|| MapCacheError::Message("map bake generation exhausted".into()))?;
        let pending = self.pending_for(descriptor, key, generation, demand, artifact)?;
        let handle = self.running_handle(handle_descriptor, demand, &pending.active);
        if let Some(previous) = jobs.pending.replace(pending) {
            *previous.active.status.lock() = MapJobStatus::Paused;
            previous.active.ready.lock().take();
        }
        drop(jobs);

        if let Some(ticket) = superseded {
            self.cancel_ticket(ticket, Cancellation::Superseded);
        }
        self.try_submit_pending(true);
        Ok(handle)
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
        let mut jobs = self.inner.jobs.lock();
        if jobs
            .active
            .as_ref()
            .is_some_and(|job| job.key == key && is_terminal(&job.status.lock()))
        {
            jobs.active.take();
        }
        drop(jobs);
        self.request(descriptor, demand)
    }

    fn cancel_ticket(&self, ticket: MapBakeTicket, reason: Cancellation) {
        if let Ok(shared) = WorkService::shared() {
            let _ = shared.lock().cancel(ticket.work_ticket(), reason);
        }
    }

    fn fail_pending(&self, message: String) {
        if let Some(pending) = self.inner.jobs.lock().pending.take() {
            *pending.active.status.lock() = MapJobStatus::Failed(message);
            pending.active.ready.lock().take();
        }
    }

    fn try_submit_pending(&self, ordinary_turn: bool) {
        let shared = match WorkService::shared() {
            Ok(shared) => shared,
            Err(error) => {
                self.fail_pending(format!("WorkAdmission unavailable: {error}"));
                return;
            }
        };
        let capacity_sequence = shared.lock().publications().capacity().sequence();
        let mut jobs = self.inner.jobs.lock();
        if jobs.active.is_some() {
            return;
        }
        let Some(mut pending) = jobs.pending.take() else {
            return;
        };
        if !ordinary_turn && pending.capacity_sequence == Some(capacity_sequence) {
            jobs.pending = Some(pending);
            return;
        }
        let job = match MapBakeJob::new(
            pending.input,
            self.inner.root.clone(),
            Arc::clone(&self.inner.producer),
            self.inner.shipped.clone(),
            pending.request,
            Arc::clone(&pending.active.demand),
        ) {
            Ok(job) => job,
            Err(error) => {
                *pending.active.status.lock() = MapJobStatus::Failed(error.to_string());
                pending.active.ready.lock().take();
                return;
            }
        };
        let input = job.input;
        let mut service = shared.lock();
        let identity = match service.mint_identity(Scope::Process, None) {
            Ok(identity) => identity,
            Err(error) => {
                *pending.active.status.lock() =
                    MapJobStatus::Failed(format!("WorkAdmission identity: {error}"));
                pending.active.ready.lock().take();
                return;
            }
        };
        let conditions = ProcessConditions::new(service.incarnation());
        match service.try_submit::<MapBakeSpec, _>(identity, &input, job, &conditions) {
            Ok(ticket) => {
                pending.active.ticket = Some(MapBakeTicket::new(ticket));
                pending.active.progress_sequence = 0;
                *pending.active.status.lock() = MapJobStatus::Queued;
                jobs.active = Some(pending.active);
            }
            Err(refused) => {
                let (refusal, job) = refused.into_parts();
                if refusal == SubmissionRefusal::Full {
                    pending.request = job.request;
                    pending.capacity_sequence = Some(service.publications().capacity().sequence());
                    jobs.pending = Some(pending);
                } else {
                    *pending.active.status.lock() =
                        MapJobStatus::Failed(format!("WorkAdmission refused map bake: {refusal}"));
                    pending.active.ready.lock().take();
                }
            }
        }
    }

    fn refresh_active(&self, ordinary_turn: bool) {
        let ticket = self
            .inner
            .jobs
            .lock()
            .active
            .as_ref()
            .and_then(|active| active.ticket);
        if let Some(ticket) = ticket {
            let shared = match WorkService::shared() {
                Ok(shared) => shared,
                Err(error) => {
                    self.fail_active_ticket(ticket, format!("WorkAdmission unavailable: {error}"));
                    return;
                }
            };
            let (progress, terminal) = {
                let mut service = shared.lock();
                let work_ticket = ticket.work_ticket();
                let progress = service.progress(&work_ticket).cloned();
                let terminal = match service.borrow_terminal(&work_ticket) {
                    Ok(Some(_)) => Some(service.consume(&work_ticket)),
                    Ok(None) => None,
                    Err(error) => Some(Err(error)),
                };
                (progress, terminal)
            };
            if let Some(terminal) = terminal {
                match terminal {
                    Ok(settlement) => self.finish_ticket(ticket, settlement),
                    Err(error) => self.fail_active_ticket(
                        ticket,
                        format!("WorkAdmission terminal custody: {error}"),
                    ),
                }
            } else if let Some(progress) = progress {
                let mut jobs = self.inner.jobs.lock();
                if let Some(active) = jobs
                    .active
                    .as_mut()
                    .filter(|active| active.ticket == Some(ticket))
                {
                    if progress.sequence() > active.progress_sequence {
                        if let Some(stage) = map_stage_from_progress(progress.phase()) {
                            active.progress_sequence = progress.sequence();
                            let completed = progress.completed();
                            let total = progress.total();
                            let message = match total {
                                Some(total) => format!("completed {completed}/{total} units"),
                                None => format!("completed {completed} units"),
                            };
                            let total = total.unwrap_or(completed);
                            *active.status.lock() = MapJobStatus::Running(MapProgress {
                                stage,
                                completed,
                                total,
                                message,
                            });
                        }
                    }
                }
            }
        }

        {
            let mut jobs = self.inner.jobs.lock();
            if jobs.active.as_ref().is_some_and(|active| {
                active.ticket.is_none()
                    && active.consumers.load(AtomicOrdering::Acquire) == 0
                    && is_terminal(&active.status.lock())
            }) {
                jobs.active.take();
            }
        }
        self.try_submit_pending(ordinary_turn);
        let mut cache = self.inner.ready.lock();
        cache.catalogues.retain(|_, ready| ready.strong_count() > 0);
        cache.images.retain(|_, ready| ready.strong_count() > 0);
        if cache.catalogues.is_empty() {
            cache.catalogues = BTreeMap::new();
        }
        if cache.images.is_empty() {
            cache.images = BTreeMap::new();
        }
    }

    fn fail_active_ticket(&self, ticket: MapBakeTicket, message: String) {
        let mut jobs = self.inner.jobs.lock();
        let Some(mut active) = jobs.active.take() else {
            return;
        };
        if active.ticket != Some(ticket) {
            jobs.active = Some(active);
            return;
        }
        active.ticket = None;
        active.descriptor = active.descriptor.clone().without_input();
        *active.status.lock() = MapJobStatus::Failed(message);
        if active.cancel_requested && active.consumers.load(AtomicOrdering::Acquire) == 0 {
            active.ready.lock().take();
        }
        if jobs.pending.is_none() {
            jobs.active = Some(active);
        }
    }

    fn finish_ticket(&self, ticket: MapBakeTicket, mut settlement: JobSettlement<MapBakeSpec>) {
        // Cache validation stays in the admitted worker. Reuse any immutable
        // owner still retained by a consumer when publishing that validated
        // identity, without nesting the cache and manager locks.
        if let Settlement::Completed(result) = &mut settlement {
            let (catalogue, images) = {
                let cache = self.inner.ready.lock();
                (
                    cache
                        .catalogues
                        .get(&result.key.catalogue)
                        .and_then(Weak::upgrade),
                    cache.images.get(&result.key.image).and_then(Weak::upgrade),
                )
            };
            if let Some(catalogue) = catalogue {
                result.ready.catalogue = catalogue;
            }
            if result.ready.images.is_some() {
                if let Some(images) = images {
                    result.ready.images = Some(images);
                }
            }
        }
        let mut ready_for_cache = None;
        let mut jobs = self.inner.jobs.lock();
        let Some(mut active) = jobs.active.take() else {
            return;
        };
        if active.ticket != Some(ticket) {
            jobs.active = Some(active);
            return;
        }
        active.ticket = None;
        let consumers = active.consumers.load(AtomicOrdering::Acquire);
        match settlement {
            Settlement::Completed(result) => {
                if result.key != active.key
                    || result.generation != active.generation
                    || result.artifact != active.artifact
                {
                    *active.status.lock() =
                        MapJobStatus::Failed("stale map bake result rejected".into());
                    active.descriptor = active.descriptor.clone().without_input();
                } else {
                    if consumers != 0 {
                        *active.ready.lock() = Some(result.ready.clone());
                        ready_for_cache = Some((active.key, result.ready));
                    }
                    let wants_images =
                        active.demand.load(AtomicOrdering::Acquire) != DEMAND_CATALOGUE;
                    if active.artifact == ArtifactKind::Catalogue
                        && wants_images
                        && !active.cancel_requested
                        && consumers != 0
                        && jobs.pending.is_none()
                    {
                        active.artifact = ArtifactKind::Images;
                        active.progress_sequence = 0;
                        *active.status.lock() = MapJobStatus::Queued;
                        match self.input_for(
                            &active.descriptor,
                            active.key,
                            active.generation,
                            ArtifactKind::Images,
                        ) {
                            Ok((input, request)) => {
                                jobs.pending = Some(PendingJob {
                                    active,
                                    input,
                                    request,
                                    capacity_sequence: None,
                                });
                            }
                            Err(error) => {
                                *active.status.lock() = MapJobStatus::Failed(error.to_string());
                                active.descriptor = active.descriptor.clone().without_input();
                                if jobs.pending.is_none() {
                                    jobs.active = Some(active);
                                }
                            }
                        }
                    } else {
                        active.descriptor = active.descriptor.clone().without_input();
                        if active.cancel_requested && consumers == 0 {
                            active.ready.lock().take();
                            *active.status.lock() = MapJobStatus::Paused;
                        } else {
                            *active.status.lock() = MapJobStatus::Ready;
                        }
                        if jobs.pending.is_none() {
                            jobs.active = Some(active);
                        }
                    }
                }
            }
            Settlement::Failed(error) => {
                *active.status.lock() = status_for_job_error(error);
                active.descriptor = active.descriptor.clone().without_input();
                if active.cancel_requested && consumers == 0 {
                    active.ready.lock().take();
                }
                if jobs.pending.is_none() {
                    jobs.active = Some(active);
                }
            }
            Settlement::Cancelled(_) => {
                *active.status.lock() = MapJobStatus::Paused;
                active.descriptor = active.descriptor.clone().without_input();
                if active.cancel_requested && consumers == 0 {
                    active.ready.lock().take();
                }
                if jobs.pending.is_none() {
                    jobs.active = Some(active);
                }
            }
        }
        drop(jobs);
        if let Some((key, ready)) = ready_for_cache {
            let mut cache = self.inner.ready.lock();
            cache
                .catalogues
                .insert(key.catalogue, Arc::downgrade(&ready.catalogue));
            if let Some(images) = &ready.images {
                cache.images.insert(key.image, Arc::downgrade(images));
            }
        }
    }

    fn cached_catalogue(
        &self,
        identity: CatalogueIdentity,
    ) -> Result<Option<Arc<ReadyCatalogue>>, MapCacheError> {
        let key = identity.key()?.0;
        let mut cache = self.inner.ready.lock();
        let Some(value) = cache.catalogues.get(&key).and_then(Weak::upgrade) else {
            cache.catalogues.remove(&key);
            return Ok(None);
        };
        Ok(Some(value))
    }

    fn cached_images(
        &self,
        identity: ImageIdentity,
    ) -> Result<Option<Arc<ReadyImages>>, MapCacheError> {
        let key = identity.key()?.0;
        let mut cache = self.inner.ready.lock();
        let Some(value) = cache.images.get(&key).and_then(Weak::upgrade) else {
            cache.images.remove(&key);
            return Ok(None);
        };
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
        let images = if demand.wants_images() {
            let Some(images) = self.cached_images(descriptor.image_identity())? else {
                return Ok(None);
            };
            Some(images)
        } else {
            None
        };
        Ok(Some(ReadyMap { catalogue, images }))
    }

    /// Whether a live lease from an admitted, preflighted job is available.
    pub fn ready_for(
        &self,
        descriptor: &MapProfileDescriptor,
        demand: MapDemand,
    ) -> Result<bool, MapCacheError> {
        Ok(self.lookup_ready(descriptor, demand)?.is_some())
    }

    /// Clone only a live lease published by an admitted, preflighted job.
    /// Disk cache hits are opened by the WorkService job, never here.
    pub fn open_ready(
        &self,
        descriptor: &MapProfileDescriptor,
        demand: MapDemand,
    ) -> Result<ReadyMap, MapCacheError> {
        let catalogue = self
            .cached_catalogue(descriptor.catalogue_identity())?
            .ok_or(MapCacheError::Message("catalogue is not ready".into()))?;
        let images = if demand.wants_images() {
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
        let mut cancelled = None;
        let mut jobs = self.inner.jobs.lock();
        if jobs
            .pending
            .as_ref()
            .is_some_and(|pending| pending.active.generation == generation)
        {
            let pending = jobs.pending.as_mut().expect("pending generation checked");
            if pending
                .active
                .consumers
                .fetch_sub(1, AtomicOrdering::AcqRel)
                == 1
            {
                let pending = jobs.pending.take().expect("pending generation checked");
                *pending.active.status.lock() = MapJobStatus::Paused;
                pending.active.ready.lock().take();
            }
            return;
        }
        if let Some(active) = jobs
            .active
            .as_mut()
            .filter(|active| active.generation == generation)
        {
            if active.consumers.fetch_sub(1, AtomicOrdering::AcqRel) == 1 {
                active.ready.lock().take();
                if !is_terminal(&active.status.lock()) && !active.cancel_requested {
                    active.cancel_requested = true;
                    cancelled = active.ticket;
                }
            }
        }
        drop(jobs);
        if let Some(ticket) = cancelled {
            self.cancel_ticket(ticket, Cancellation::Operator);
        }
    }

    /// Advance the manager's view of shared WorkAdmission publications and
    /// retry bounded pending demand on this ordinary manager turn.
    pub fn reap(&self) {
        self.refresh_active(true);
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

fn status_for_job_error(error: JobError<MapCacheError>) -> MapJobStatus {
    match error {
        JobError::Family(MapCacheError::Cancelled) => MapJobStatus::Paused,
        error => MapJobStatus::Failed(error.to_string()),
    }
}

/// Atomically publish a finished partial directory as `ready`.
fn publish_partial(partial: &Path, ready: &Path) -> Result<(), MapCacheError> {
    run_once(|| fs::rename(partial, ready))?;
    sync_directory(ready.parent().ok_or(MapCacheError::Map(MapError::Path))?)?;
    clear_checkpoint(ready)
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
    /// Terrain lease kept from an admission decision (see
    /// [`Self::with_terrain_pin`]).
    terrain_pin: Option<Arc<ReadyImages>>,
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
            terrain_pin: None,
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
            terrain_pin: None,
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
            terrain_pin: None,
        }
    }

    pub fn demand(&self) -> MapDemand {
        self.demand
    }
    pub fn generation(&self) -> Option<u64> {
        self.generation
    }
    /// Ready without cloning the status (no allocation on a UI frame).
    pub fn is_ready(&self) -> bool {
        self.status() == MapJobStatus::Ready
    }
    pub fn status(&self) -> MapJobStatus {
        self.manager.reap();
        self.status.lock().clone()
    }
    /// Wait for one shared WorkAdmission publication without holding the
    /// manager or UI state lock. A timeout is a normal poll boundary.
    pub fn wait_for_change(
        &self,
        timeout: Duration,
    ) -> script::work_admission::PublicationWaitOutcome {
        let Ok(shared) = WorkService::shared() else {
            return script::work_admission::PublicationWaitOutcome::Unavailable;
        };
        let observed = shared.lock().publication_epoch();
        self.manager.reap();
        if is_terminal(&self.status.lock()) {
            return script::work_admission::PublicationWaitOutcome::Changed;
        }
        WorkService::wait_for_publication_change(&shared, observed, timeout)
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
    /// Keep `terrain` leased for this handle's lifetime, so the terrain an
    /// admission decision relied on cannot be pruned while the worker derives
    /// the catalogue and adopts it.
    pub fn with_terrain_pin(mut self, terrain: Arc<ReadyImages>) -> Self {
        self.terrain_pin = Some(terrain);
        self
    }
}

impl Clone for MapDemandHandle {
    fn clone(&self) -> Self {
        let mut clone = self.clone_demand();
        clone.terrain_pin = self.terrain_pin.as_ref().map(Arc::clone);
        clone
    }
}

impl MapDemandHandle {
    fn clone_demand(&self) -> Self {
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
                terrain_pin: self.terrain_pin.as_ref().map(Arc::clone),
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

mod shipped;
use shipped::ShippedSource;
pub use shipped::{ShippedMapImages, SHIPPED_IMAGES_NAME};

#[cfg(any(test, feature = "test-support"))]
pub mod fixture;

#[cfg(test)]
mod tests {
    use super::fixture::{fixture_png, synthetic_client_snapshot, wait_for_status};
    use super::*;
    use nav::map::formats::{ColorFormat, Coverage, CoverageLevel, ImageManifest, PlaneBounds};
    use nav::map::identity::IMAGE_SCHEMA;
    use nav::map::poi::PoiRecord;
    use nav::map::spatial::{TileKey, WorldBounds, TILE_GUTTER, TILE_INTERIOR};
    use std::io::{BufRead, BufReader, Read};
    use std::process::{Command, Stdio};
    use std::sync::atomic::AtomicUsize;
    use std::time::Instant;

    #[test]
    fn map_ticket_reference_preserves_identity_and_revision() {
        use api::identity::{ProcessIncarnation, RequestIdMinter};
        use std::num::NonZeroU64;

        let incarnation = ProcessIncarnation::from_start_nonce(NonZeroU64::new(1).unwrap());
        let identity = WorkIdentity::new(
            RequestIdMinter::new().mint().unwrap(),
            incarnation,
            Scope::Process,
            None,
        )
        .unwrap();
        let ticket = WorkTicket::<MapBakeSpec>::new(identity, InputRevision::new(7));
        let stored = MapBakeTicket::new(ticket);
        assert_eq!(stored.work_ticket(), ticket);
        assert_ne!(
            stored,
            MapBakeTicket::new(WorkTicket::new(identity, InputRevision::new(8)))
        );
    }

    #[test]
    fn dropping_manager_releases_terminal_capacity() {
        const CHILD_ROOT: &str = "274BOT_MAP_DROP_MANAGER_CHILD_ROOT";
        let Some(path) = std::env::var_os(CHILD_ROOT) else {
            let root = temp_root("drop-manager-capacity");
            let child = spawn_test_child(
                "map_cache::tests::dropping_manager_releases_terminal_capacity",
                (CHILD_ROOT, root.path()),
            );
            assert_children_succeeded(vec![child]);
            return;
        };
        // A fresh child isolates this real process-wide ledger from other tests.
        let root = MapCacheRoot::from_existing_root(path);
        let shared = WorkService::shared().unwrap();
        let baseline = shared.lock().capacity();
        let manager = MapDemandManager::new(root.clone(), FixtureProducer::new(false));
        let handle = manager
            .request(descriptor(&root), MapDemand::CatalogueOnly)
            .unwrap();
        let ticket = manager
            .inner
            .jobs
            .lock()
            .active
            .as_ref()
            .and_then(|active| active.ticket)
            .expect("the isolated map job is admitted");
        let started = Instant::now();
        loop {
            // Do not poll the manager: leave its terminal unconsumed on purpose.
            let (settled, observed) = {
                let service = shared.lock();
                (
                    service
                        .borrow_terminal(&ticket.work_ticket())
                        .unwrap()
                        .is_some(),
                    service.publication_epoch(),
                )
            };
            if settled {
                break;
            }
            assert!(
                started.elapsed() < Duration::from_secs(120),
                "the isolated map job never settled"
            );
            WorkService::wait_for_publication_change(&shared, observed, Duration::from_millis(100));
        }
        assert!(shared.lock().capacity().jobs() > baseline.jobs());
        drop(handle);
        drop(manager);
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let (released, observed) = {
                let service = shared.lock();
                service.observe_health();
                let observed = service.publication_epoch();
                (service.capacity() == baseline, observed)
            };
            if released {
                break;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "the last manager must eventually abandon its unconsumed terminal custody"
            );
            assert_ne!(
                WorkService::wait_for_publication_change(&shared, observed, remaining),
                script::work_admission::PublicationWaitOutcome::Unavailable,
                "WorkAdmission became unavailable before the abandoned map job released capacity"
            );
        }
        assert_eq!(shared.lock().capacity(), baseline);
    }

    fn temp_root(name: &str) -> MapCacheRoot {
        let path =
            std::env::temp_dir().join(format!("274bot-map-cache-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        MapCacheRoot::from_root(path)
    }

    /// `MapStage` → the frozen `api::work::MapBakePhase`, in order.
    fn frozen_phase(stage: MapStage) -> api::work::MapBakePhase {
        use api::work::MapBakePhase;
        match stage {
            MapStage::ReadingCache => MapBakePhase::ReadingCache,
            MapStage::DerivingPois => MapBakePhase::DerivingPois,
            MapStage::BakingPlane => MapBakePhase::BakingPlane,
            MapStage::BuildingZoomLevels => MapBakePhase::BuildingZoomLevels,
            MapStage::Publishing => MapBakePhase::Publishing,
        }
    }

    /// `MapJobStatus` → the frozen `api::work::MapBakeStatus`, variant for
    /// variant.
    fn frozen_status(status: &MapJobStatus) -> api::work::MapBakeStatus {
        use api::work::MapBakeStatus;
        match status {
            MapJobStatus::Queued => MapBakeStatus::Queued,
            MapJobStatus::Running(_) => MapBakeStatus::Running,
            MapJobStatus::Ready => MapBakeStatus::Ready,
            MapJobStatus::Paused => MapBakeStatus::Paused,
            MapJobStatus::Cancelled => MapBakeStatus::Cancelled,
            MapJobStatus::Failed(_) => MapBakeStatus::Failed,
        }
    }

    /// F0-4 (§26 §2.2, §2.6): the frozen map-bake family characterizes this
    /// producer's real stages, endings and cancellation observation.
    #[test]
    fn frozen_map_bake_family_characterizes_this_producer() {
        use api::work::{MapBakePhase, MapBakeStatus, WorkFamily, WorkPhase};

        let stages = [
            MapStage::ReadingCache,
            MapStage::DerivingPois,
            MapStage::BakingPlane,
            MapStage::BuildingZoomLevels,
            MapStage::Publishing,
        ];
        assert_eq!(stages.len(), MapBakePhase::ALL.len());
        assert_eq!(stages.len(), WorkFamily::MapBake.phases().len());
        for ((index, stage), phase) in stages.iter().enumerate().zip(MapBakePhase::ALL) {
            assert_eq!(frozen_phase(*stage), phase);
            assert_eq!(map_bake_phase(*stage), phase);
            assert_eq!(map_stage_from_progress(phase.index()), Some(*stage));
            assert_eq!(usize::from(WorkPhase::index(phase)), index);
            assert_eq!(WorkFamily::MapBake.phases()[index], format!("{stage:?}"));
        }
        assert_eq!(map_stage_from_progress(u16::MAX), None);

        let statuses = [
            MapJobStatus::Queued,
            MapJobStatus::Running(MapProgress {
                stage: MapStage::BakingPlane,
                completed: 1,
                total: 4,
                message: String::new(),
            }),
            MapJobStatus::Ready,
            MapJobStatus::Paused,
            MapJobStatus::Cancelled,
            MapJobStatus::Failed(String::from("boom")),
        ];
        assert_eq!(statuses.len(), MapBakeStatus::ALL.len());
        for (status, frozen) in statuses.iter().zip(MapBakeStatus::ALL) {
            assert_eq!(frozen_status(status), frozen);
            assert_eq!(
                is_terminal(status),
                frozen.is_domain_ending(),
                "is_terminal({status:?}) must agree with the frozen ending"
            );
        }

        // A producer cancellation is a paused family outcome, while any other
        // family or service failure remains a domain failure.
        let paused = status_for_job_error(JobError::Family(MapCacheError::Cancelled));
        assert_eq!(paused, MapJobStatus::Paused);
        assert!(frozen_status(&paused).is_family_result());
        let failed = status_for_job_error(JobError::Family(MapCacheError::Busy));
        assert!(
            matches!(&failed, MapJobStatus::Failed(message) if message == &MapCacheError::Busy.to_string())
        );
        assert_eq!(frozen_status(&failed), MapBakeStatus::Failed);
        let failed_internally = status_for_job_error(JobError::FailedInternally);
        assert!(matches!(failed_internally, MapJobStatus::Failed(_)));
        // WorkAdmission cancellation uses Settlement::Cancelled and is not
        // fabricated as a producer error.
        // Nothing in this producer constructs `Cancelled`.
        assert!(!MapBakeStatus::Cancelled.is_produced_today());
        assert!(WorkFamily::MapBake
            .cancellation_mapping()
            .contains("MapJobStatus::Paused"));
    }

    #[test]
    fn cursor_progress_coalesces_and_keeps_meaningful_boundaries() {
        let reading = MapProgressSnapshot {
            stage: MapStage::ReadingCache,
            completed: 0,
            total: None,
        };
        assert!(should_publish_cursor_progress(None, reading, false));
        assert!(!should_publish_cursor_progress(
            Some(reading),
            MapProgressSnapshot {
                completed: MAP_PROGRESS_UNKNOWN_STEP - 1,
                ..reading
            },
            false,
        ));
        assert!(should_publish_cursor_progress(
            Some(reading),
            MapProgressSnapshot {
                completed: MAP_PROGRESS_UNKNOWN_STEP,
                ..reading
            },
            false,
        ));

        let terrain = MapProgressSnapshot {
            stage: MapStage::BakingPlane,
            completed: 0,
            total: Some(1_000),
        };
        assert!(should_publish_cursor_progress(
            Some(reading),
            terrain,
            false
        ));
        assert!(!should_publish_cursor_progress(
            Some(terrain),
            MapProgressSnapshot {
                completed: 9,
                ..terrain
            },
            false,
        ));
        assert!(should_publish_cursor_progress(
            Some(terrain),
            MapProgressSnapshot {
                completed: 10,
                ..terrain
            },
            false,
        ));
        assert!(should_publish_cursor_progress(Some(terrain), terrain, true));
        assert!(cursor_progress_boundary(Some(reading), terrain, false));
        assert!(!cursor_progress_boundary(
            Some(terrain),
            MapProgressSnapshot {
                completed: 10,
                ..terrain
            },
            false
        ));
        assert!(cursor_progress_boundary(Some(terrain), terrain, true));

        assert_eq!(
            normalize_cursor_progress(MapProgressSnapshot {
                stage: MapStage::ReadingCache,
                completed: 0,
                total: Some(0),
            })
            .total,
            None
        );
    }

    #[test]
    fn joined_demand_never_revokes_terrain_bake_consent() {
        for initial in [DEMAND_CATALOGUE, DEMAND_ADOPT_IMAGES, DEMAND_BAKE_IMAGES] {
            for demand in [
                MapDemand::CatalogueOnly,
                MapDemand::ReadyImages,
                MapDemand::Images,
            ] {
                let flag = AtomicUsize::new(initial);
                upgrade_demand(&flag, demand);
                let expected = match (initial, demand) {
                    (_, MapDemand::Images) | (DEMAND_BAKE_IMAGES, _) => DEMAND_BAKE_IMAGES,
                    (DEMAND_ADOPT_IMAGES, _) | (_, MapDemand::ReadyImages) => DEMAND_ADOPT_IMAGES,
                    _ => DEMAND_CATALOGUE,
                };
                assert_eq!(flag.load(AtomicOrdering::Acquire), expected);
            }
        }
    }

    fn descriptor(root: &MapCacheRoot) -> MapProfileDescriptor {
        let _ = root;
        MapProfileDescriptor::new(274, Digest([1; 32]), Digest([2; 32]), Digest([3; 32])).unwrap()
    }

    struct FixtureProducer {
        rasterized: Arc<AtomicUsize>,
        runs: AtomicUsize,
        interrupt_once: Arc<AtomicBool>,
    }

    type WrittenTiles = Arc<Mutex<Vec<(TileKey, u64)>>>;

    struct FixtureCursor {
        artifact: ArtifactKind,
        catalogue_identity: CatalogueIdentity,
        image_identity: ImageIdentity,
        partial_dir: Arc<PathBuf>,
        plan: BakePlan,
        progress: MapProgressSnapshot,
        keys: Vec<TileKey>,
        next_tile: usize,
        tiles: Vec<TileReceipt>,
        completed: Vec<CompletedUnit>,
        rasterized: Arc<AtomicUsize>,
        interrupt_once: Arc<AtomicBool>,
        written: Option<WrittenTiles>,
    }

    fn fixture_charge() -> WorkCharge {
        WorkCharge::new(64 * 1024, 1024 * 1024, 1024 * 1024, 0)
    }

    impl FixtureCursor {
        fn new(
            request: &BakeRequest,
            partial_dir: Arc<PathBuf>,
            keys: Vec<TileKey>,
            rasterized: Arc<AtomicUsize>,
            interrupt_once: Arc<AtomicBool>,
            written: Option<WrittenTiles>,
        ) -> Self {
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
                        planned_units: keys.len() as u32,
                        stage: BakeStage::BaseTerrain,
                    },
                    MapProgressSnapshot {
                        stage: MapStage::BakingPlane,
                        completed: 0,
                        total: Some(keys.len() as u32),
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
                keys,
                next_tile: 0,
                tiles: Vec::new(),
                completed: Vec::new(),
                rasterized,
                interrupt_once,
                written,
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
                    unresolved: nav::map::Rows::new(Vec::new())?,
                },
                records: nav::map::Rows::<PoiRecord, 4096>::new(Vec::new())?,
            })
        }

        fn image_manifest(&mut self) -> Result<ImageManifest, MapCacheError> {
            Ok(ImageManifest {
                schema: IMAGE_SCHEMA,
                identity: self.image_identity,
                key: self.image_identity.key()?,
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
                tiles: nav::map::Rows::new(std::mem::take(&mut self.tiles))?,
            })
        }

        fn consume(&mut self, context: &mut StepContext<'_, MapBakePhase>) -> bool {
            match context.consume_unit() {
                WorkCheckpoint::Cancel(_) => false,
                WorkCheckpoint::Continue | WorkCheckpoint::Yield => true,
            }
        }
    }

    impl MapBakeCursor for FixtureCursor {
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
                    if !self.consume(context) {
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
                    let Some(key) = self.keys.get(self.next_tile).copied() else {
                        return Err(MapCacheError::Map(MapError::Invalid(
                            "fixture cursor exhausted",
                        )));
                    };
                    let unit = UnitKey::Terrain { tile: key };
                    let (receipt, rasterized_this_step) = match writer.completed_receipt(unit) {
                        Some(receipt) => (receipt, false),
                        None => {
                            let payload = fixture_png(self.next_tile as u8 + 1);
                            let receipt = writer.write_image_tile(unit, &payload)?;
                            let path = self.partial_dir.join(key.relative_path()?);
                            self.completed.push(CompletedUnit {
                                key: unit,
                                payload: receipt,
                            });
                            if let Some(written) = &self.written {
                                written.lock().push((key, file_id(&path)));
                            }
                            self.rasterized.fetch_add(1, AtomicOrdering::Relaxed);
                            (receipt, true)
                        }
                    };
                    self.tiles.push(TileReceipt {
                        key,
                        payload: receipt,
                    });
                    self.next_tile += 1;
                    self.progress.completed = self.next_tile as u32;
                    let consumed = self.consume(context);
                    if !consumed {
                        return Ok(MapBakeCursorStep::Cancelled);
                    }
                    if rasterized_this_step
                        && self.interrupt_once.swap(false, AtomicOrdering::AcqRel)
                    {
                        return Err(MapCacheError::Cancelled);
                    }
                    if self.next_tile == self.keys.len() {
                        Ok(MapBakeCursorStep::Completed(MapBakeCursorOutput::Images(
                            self.image_manifest()?,
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

    impl FixtureProducer {
        fn new(interrupt_once: bool) -> Arc<Self> {
            Arc::new(Self {
                rasterized: Arc::new(AtomicUsize::new(0)),
                runs: AtomicUsize::new(0),
                interrupt_once: Arc::new(AtomicBool::new(interrupt_once)),
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
    }

    impl MapBakeProducer for FixtureProducer {
        fn charge(&self, _request: &BakeRequest) -> Result<WorkCharge, MapCacheError> {
            Ok(fixture_charge())
        }

        fn cursor(
            &self,
            request: &BakeRequest,
            partial_dir: Arc<PathBuf>,
        ) -> Result<Box<dyn MapBakeCursor>, MapCacheError> {
            self.runs.fetch_add(1, AtomicOrdering::Relaxed);
            Ok(Box::new(FixtureCursor::new(
                request,
                partial_dir,
                Self::tile_keys().to_vec(),
                Arc::clone(&self.rasterized),
                Arc::clone(&self.interrupt_once),
                None,
            )))
        }
    }
    fn fixture_cursor(request: &BakeRequest) -> FixtureCursor {
        FixtureCursor::new(
            request,
            Arc::new(request.partial_dir().to_path_buf()),
            FixtureProducer::tile_keys().to_vec(),
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicBool::new(false)),
            None,
        )
    }

    fn fixture_writer(
        root: &MapCacheRoot,
        descriptor: MapProfileDescriptor,
        artifact: ArtifactKind,
        plan: BakePlan,
    ) -> (BakeRequest, BakeWriter) {
        let request = bake_request(root, descriptor, artifact).unwrap();
        let writer = BakeWriter::open(
            root.clone(),
            &request,
            plan,
            Arc::new(AtomicBool::new(false)),
            Arc::new(|_| {}),
        )
        .unwrap();
        (request, writer)
    }

    /// Cursor fixture for proving that already-written native raster files
    /// survive checkpointing and publication without replacement.
    struct ResumeAdoptProducer {
        rasterized: Arc<AtomicUsize>,
        interrupt_once: Arc<AtomicBool>,
        written: Arc<Mutex<Vec<(TileKey, u64)>>>,
    }

    impl ResumeAdoptProducer {
        fn new(interrupt: bool) -> Arc<Self> {
            Arc::new(Self {
                rasterized: Arc::new(AtomicUsize::new(0)),
                interrupt_once: Arc::new(AtomicBool::new(interrupt)),
                written: Arc::new(Mutex::new(Vec::new())),
            })
        }

        fn keys() -> [TileKey; 3] {
            [
                TileKey {
                    plane: 0,
                    lod: 0,
                    x: 50,
                    z: 50,
                },
                TileKey {
                    plane: 0,
                    lod: 0,
                    x: 51,
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
    }

    impl MapBakeProducer for ResumeAdoptProducer {
        fn charge(&self, _request: &BakeRequest) -> Result<WorkCharge, MapCacheError> {
            Ok(fixture_charge())
        }

        fn cursor(
            &self,
            request: &BakeRequest,
            partial_dir: Arc<PathBuf>,
        ) -> Result<Box<dyn MapBakeCursor>, MapCacheError> {
            Ok(Box::new(FixtureCursor::new(
                request,
                partial_dir,
                Self::keys().to_vec(),
                Arc::clone(&self.rasterized),
                Arc::clone(&self.interrupt_once),
                Some(Arc::clone(&self.written)),
            )))
        }
    }

    fn wait_ready(handle: &MapDemandHandle) {
        wait_for_status(handle, MapJobStatus::Ready);
    }

    fn complete_census(root: &MapCacheRoot) -> CacheCensusResult {
        let mut cursor = CacheCensusCursor::new(root.clone());
        loop {
            if let Some(census) = cursor.advance().unwrap() {
                return census;
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn artifact_census_counts_many_symlinks_without_following_or_removing_them() {
        use std::os::unix::fs::symlink;

        let root = temp_root("many-symlink-census");
        let descriptor = descriptor(&root);
        let job = active_fixture_job(&root, descriptor.clone());
        let partial = job.request.partial_dir();
        let external = std::env::temp_dir().join(format!(
            "274bot-map-cache-symlink-target-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&external);
        fs::create_dir_all(&external).unwrap();
        let target_file = external.join("keep.txt");
        fs::write(&target_file, b"keep").unwrap();

        let root_id = ArtifactRootId {
            revision: descriptor.revision(),
            kind: ArtifactKind::Images,
            key: descriptor.image_key().unwrap(),
            partial: true,
        };
        let baseline_entries = complete_census(&root)
            .roots
            .into_iter()
            .find(|summary| summary.id == root_id)
            .unwrap()
            .entries;
        symlink(external.as_path(), partial.join("foreign-0000")).unwrap();
        let single_link_census = complete_census(&root);
        let single_link_summary = single_link_census
            .roots
            .iter()
            .find(|summary| summary.id == root_id)
            .unwrap();
        assert_eq!(single_link_summary.entries, baseline_entries + 1);
        assert!(!single_link_summary.safe_to_prune);
        for index in 1..=MAX_ARTIFACT_ROOT_ENTRIES {
            symlink(
                target_file.as_path(),
                partial.join(format!("foreign-{index:04}")),
            )
            .unwrap();
        }
        let census = complete_census(&root);
        assert_eq!(
            census.overrun,
            Some(CacheCensusOverrun::ArtifactRootEntries)
        );
        let summary = census
            .roots
            .iter()
            .find(|summary| summary.id == root_id)
            .unwrap();
        assert_eq!(summary.entries, MAX_ARTIFACT_ROOT_ENTRIES + 1);
        assert!(!summary.safe_to_prune);
        assert!(fs::symlink_metadata(partial.join("foreign-0000"))
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(&target_file).unwrap(), b"keep");

        drop(job);
        fs::remove_dir_all(root.path()).unwrap();
        fs::remove_dir_all(external).unwrap();
    }

    fn active_fixture_job(root: &MapCacheRoot, descriptor: MapProfileDescriptor) -> MapBakeJob {
        let request = bake_request(root, descriptor.clone(), ArtifactKind::Images).unwrap();
        let producer = FixtureProducer::new(false);
        let input = MapBakeInput {
            key: JobKey {
                catalogue: descriptor.catalogue_key().unwrap(),
                image: descriptor.image_key().unwrap(),
            },
            generation: 1,
            artifact: ArtifactKind::Images,
            charge: producer.charge(&request).unwrap(),
        };
        let mut job = MapBakeJob::new(
            input,
            root.clone(),
            producer,
            None,
            request,
            Arc::new(AtomicUsize::new(DEMAND_BAKE_IMAGES)),
        )
        .unwrap();
        assert!(job.prepare_paths().unwrap());
        job.census_result = Some(complete_census(root));
        let ExclusiveAcquire::Locked(lock) = job.acquire_exclusive().unwrap() else {
            panic!("fixture writer must acquire its fresh census");
        };
        assert!(job.begin_cursor().unwrap());
        let plan = job.cursor.as_ref().unwrap().plan().unwrap();
        job.open_writer(plan).unwrap();
        drop(lock);
        job.stage = MapBakeJobStage::Cursor;
        job
    }

    #[cfg(not(windows))]
    #[test]
    fn absent_terrain_probe_does_not_create_paths_or_sync_directories() {
        let root = temp_root("read-only-terrain-probe");
        let descriptor = descriptor(&root);
        let request = bake_request(&root, descriptor.clone(), ArtifactKind::Images).unwrap();
        let producer = FixtureProducer::new(false);
        let input = MapBakeInput {
            key: JobKey {
                catalogue: descriptor.catalogue_key().unwrap(),
                image: descriptor.image_key().unwrap(),
            },
            generation: 1,
            artifact: ArtifactKind::Images,
            charge: producer.charge(&request).unwrap(),
        };
        let demand = Arc::new(AtomicUsize::new(DEMAND_ADOPT_IMAGES));
        let mut job = MapBakeJob::new(
            input,
            root.clone(),
            producer,
            None,
            request,
            Arc::clone(&demand),
        )
        .unwrap();
        TEST_DIRECTORY_SYNCS.with(|trace| *trace.borrow_mut() = Some(Vec::new()));
        assert!(job.prepare_paths().unwrap());
        let synced = TEST_DIRECTORY_SYNCS.with(|trace| trace.borrow_mut().take().unwrap());
        assert!(synced.is_empty(), "{synced:?}");
        assert!(!root.path().exists());
        assert!(!job.paths_prepared);

        demand.store(DEMAND_BAKE_IMAGES, AtomicOrdering::Release);
        assert!(job.prepare_paths().unwrap());
        assert!(job.paths_prepared);
        assert!(root
            .kind_dir(descriptor.revision(), ArtifactKind::Images)
            .is_dir());
        drop(job);
        fs::remove_dir_all(root.path()).unwrap();
    }

    #[test]
    fn catalogue_writer_defers_empty_checkpoint_until_payload_is_ready() {
        let root = temp_root("defer-empty-catalogue-checkpoint");
        let descriptor = descriptor(&root);
        let request = bake_request(&root, descriptor.clone(), ArtifactKind::Catalogue).unwrap();
        let payload = fixture_cursor(&request)
            .catalogue()
            .unwrap()
            .encode()
            .unwrap();
        let producer = FixtureProducer::new(false);
        let input = MapBakeInput {
            key: JobKey {
                catalogue: descriptor.catalogue_key().unwrap(),
                image: descriptor.image_key().unwrap(),
            },
            generation: 1,
            artifact: ArtifactKind::Catalogue,
            charge: producer.charge(&request).unwrap(),
        };
        let mut job = MapBakeJob::new(
            input,
            root.clone(),
            producer,
            None,
            request,
            Arc::new(AtomicUsize::new(DEMAND_CATALOGUE)),
        )
        .unwrap();
        assert!(job.prepare_paths().unwrap());
        job.census_result = Some(complete_census(&root));
        let ExclusiveAcquire::Locked(lock) = job.acquire_exclusive().unwrap() else {
            panic!("fresh catalogue writer must acquire its census");
        };
        job.open_writer(BakePlan {
            planned_units: 1,
            stage: BakeStage::Catalogue,
        })
        .unwrap();
        let path = job
            .writer
            .as_ref()
            .unwrap()
            .directory()
            .join("checkpoint.json");
        job.persist_checkpoint().unwrap();
        assert!(!path.exists(), "an empty checkpoint preserves no payload");
        #[cfg(not(windows))]
        TEST_DIRECTORY_SYNCS.with(|trace| *trace.borrow_mut() = Some(Vec::new()));
        job.writer
            .as_mut()
            .unwrap()
            .publish_unit(UnitKey::ClientPois, &payload)
            .unwrap();
        #[cfg(not(windows))]
        {
            let synced = TEST_DIRECTORY_SYNCS.with(|trace| trace.borrow_mut().take().unwrap());
            assert!(
                synced.is_empty(),
                "the checkpoint must persist the catalogue rename without a separate directory sync: {synced:?}"
            );
            TEST_DIRECTORY_SYNCS.with(|trace| *trace.borrow_mut() = Some(Vec::new()));
        }
        job.persist_checkpoint().unwrap();
        #[cfg(not(windows))]
        {
            let synced = TEST_DIRECTORY_SYNCS.with(|trace| trace.borrow_mut().take().unwrap());
            assert_eq!(synced, vec![path.parent().unwrap().to_path_buf()]);
        }
        let checkpoint_bytes = fs::read(&path).unwrap();
        let checkpoint = Checkpoint::decode(
            &checkpoint_bytes,
            ArtifactIdentity::Catalogue(descriptor.catalogue_identity()),
        )
        .unwrap();
        assert_eq!(checkpoint.completed.as_slice().len(), 1);
        assert_eq!(checkpoint.completed.as_slice()[0].key, UnitKey::ClientPois);
        #[cfg(not(windows))]
        TEST_DIRECTORY_SYNCS.with(|trace| *trace.borrow_mut() = Some(Vec::new()));
        let identity = descriptor.catalogue_identity();
        let manifest = CatalogueManifest {
            schema: CATALOGUE_SCHEMA,
            identity,
            key: identity.key().unwrap(),
            record_count: ClientPois::decode(&payload, identity)
                .unwrap()
                .records
                .as_slice()
                .len()
                .try_into()
                .unwrap(),
            payload: checkpoint.completed.as_slice()[0].payload,
        };
        let partial = job
            .writer
            .take()
            .unwrap()
            .finish(BakeOutput::Catalogue(manifest))
            .unwrap();
        assert_eq!(
            fs::read(&path).unwrap(),
            checkpoint_bytes,
            "the completion manifest supersedes the checkpoint without rewriting it"
        );
        #[cfg(not(windows))]
        {
            let synced = TEST_DIRECTORY_SYNCS.with(|trace| trace.borrow_mut().take().unwrap());
            assert_eq!(synced, vec![partial.as_ref().clone()]);
        }
        CatalogueManifest::decode(&fs::read(partial.join("manifest.json")).unwrap(), identity)
            .unwrap()
            .read_payload(&payload)
            .unwrap();
        drop((lock, job));
        fs::remove_dir_all(root.path()).unwrap();
    }

    #[test]
    fn plan_discovery_does_not_lock_cache_before_output() {
        use api::identity::Deadline;
        use api::work::{SharedCancellation, SliceBudget, SystemClock};
        use std::num::NonZeroU32;

        struct PlanningCursor {
            plan: Option<BakePlan>,
            inner: Box<dyn MapBakeCursor>,
            output_entered: Arc<AtomicBool>,
        }

        impl MapBakeCursor for PlanningCursor {
            fn plan(&self) -> Option<BakePlan> {
                self.plan
            }

            fn step(
                &mut self,
                context: &mut StepContext<'_, MapBakePhase>,
                writer: Option<&mut BakeWriter>,
            ) -> Result<MapBakeCursorStep, MapCacheError> {
                if self.plan.is_some() {
                    if writer.is_some() {
                        self.output_entered.store(true, AtomicOrdering::Release);
                    }
                    return self.inner.step(context, writer);
                }
                assert!(writer.is_none(), "plan discovery cannot write cache output");
                self.plan = self.inner.plan();
                Ok(match context.consume_unit() {
                    WorkCheckpoint::Cancel(_) => MapBakeCursorStep::Cancelled,
                    WorkCheckpoint::Continue | WorkCheckpoint::Yield => MapBakeCursorStep::Yielded,
                })
            }

            fn progress(&self) -> MapProgressSnapshot {
                self.inner.progress()
            }

            fn take_completed_units(&mut self) -> Vec<CompletedUnit> {
                self.inner.take_completed_units()
            }
        }

        let root = temp_root("input-only-plan");
        let mut job = active_fixture_job(&root, descriptor(&root));
        let inner = job.cursor.take().unwrap();
        let output_entered = Arc::new(AtomicBool::new(false));
        job.writer = None;
        job.census_result = Some(complete_census(&root));
        job.cursor = Some(Box::new(PlanningCursor {
            plan: None,
            inner,
            output_entered: Arc::clone(&output_entered),
        }));
        let clock = SystemClock;
        let cancellation = SharedCancellation::new();
        let step = |job: &mut MapBakeJob| {
            let budget = SliceBudget::new(
                NonZeroU32::new(64).unwrap(),
                Deadline::after(Instant::now(), Duration::from_secs(60)),
            );
            job.step(&mut StepContext::new(&clock, budget, &cancellation))
        };

        let capacity = root.try_acquire_capacity().unwrap().unwrap();
        let key = PublishLock::try_acquire(&job.lock_path).unwrap().unwrap();
        assert!(matches!(step(&mut job), StepOutcome::Yielded));
        assert!(
            job.cursor.as_ref().unwrap().plan().is_some(),
            "input-only plan discovery must advance while another cache owner holds both locks"
        );
        assert!(job.writer.is_none(), "plan exposure must not open a writer");
        assert!(matches!(step(&mut job), StepOutcome::Yielded));
        assert!(job.writer.is_none(), "output must wait for cache ownership");
        drop(key);
        drop(capacity);
        let generation = job.census_result.as_ref().unwrap().generation;

        assert!(matches!(
            step(&mut job),
            StepOutcome::Yielded | StepOutcome::Completed(_)
        ));
        assert!(
            output_entered.load(AtomicOrdering::Acquire),
            "output starts after locked admission"
        );
        assert!(matches!(job.stage, MapBakeJobStage::Serialize));
        assert_eq!(
            job.census_result.as_ref().unwrap().generation,
            generation + 1,
            "one cursor batch acquires mutation ownership once, not once per unit"
        );
    }

    #[test]
    fn census_refresh_preserves_cursor_and_each_publication_stage() {
        let root = temp_root("census-resume-stage");
        let mut job = active_fixture_job(&root, descriptor(&root));
        let cursor = job.cursor.as_ref().unwrap().as_ref() as *const dyn MapBakeCursor;
        for stage in [
            MapBakeJobStage::Cursor,
            MapBakeJobStage::Serialize,
            MapBakeJobStage::Finish,
            MapBakeJobStage::Publish,
            MapBakeJobStage::OpenPublished,
        ] {
            job.stage = stage;
            job.start_census();
            assert_eq!(job.stage, MapBakeJobStage::Census);
            job.accept_census(complete_census(&root)).unwrap();
            assert_eq!(job.stage, stage);
            assert!(std::ptr::eq(
                cursor,
                job.cursor.as_ref().unwrap().as_ref() as *const dyn MapBakeCursor
            ));
        }
        drop(job);
        fs::remove_dir_all(root.path()).unwrap();
    }

    #[test]
    fn census_refresh_rebases_live_writer_whole_cache_capacity() {
        let root = temp_root("census-rebase-writer");
        let mut first = active_fixture_job(&root, descriptor(&root));
        let previous_bytes = first.writer.as_ref().unwrap().cache_bytes;
        let second_descriptor =
            MapProfileDescriptor::new(274, Digest([41; 32]), Digest([42; 32]), Digest([43; 32]))
                .unwrap();
        let second = active_fixture_job(&root, second_descriptor);
        first.start_census();
        let census = complete_census(&root);
        assert!(census.bytes > previous_bytes);
        let expected_bytes = census.bytes;
        let expected_entries = census.entries;
        let expected_product_entries = census.product_entries;
        first.accept_census(census).unwrap();
        let writer = first.writer.as_ref().unwrap();
        assert_eq!(writer.cache_bytes, expected_bytes);
        assert_eq!(writer.cache_entries, expected_entries);
        assert_eq!(writer.cache_product_entries, expected_product_entries);
        assert_eq!(first.stage, MapBakeJobStage::Cursor);
        drop((first, second));
        fs::remove_dir_all(root.path()).unwrap();
    }

    #[test]
    fn census_refresh_restarts_a_concurrently_pruned_unpublished_partial() {
        let root = temp_root("census-pruned-live-partial");
        let mut job = active_fixture_job(&root, descriptor(&root));
        job.stage = MapBakeJobStage::Serialize;
        job.start_census();
        let census = complete_census(&root);
        let summary = census
            .roots
            .iter()
            .find(|summary| summary.id.partial)
            .unwrap();
        let mut prune = PruneCursor::new(root.clone(), summary, census.generation).unwrap();
        finish_prune(&mut prune);
        job.accept_census(complete_census(&root)).unwrap();
        assert_eq!(job.stage, MapBakeJobStage::OpenExisting);
        assert!(job.writer.is_none());
        assert!(job.cursor.is_none());
        assert!(job.cursor_output.is_none());
        assert!(job.bake_output.is_none());
        drop(job);
        fs::remove_dir_all(root.path()).unwrap();
    }

    #[test]
    fn clean_cursor_yields_do_not_rewrite_a_durable_checkpoint() {
        let root = temp_root("clean-checkpoint-yield");
        let mut job = active_fixture_job(&root, descriptor(&root));
        let writer = job.writer.as_mut().unwrap();
        let path = writer.directory().join("checkpoint.json");
        fs::File::open(&path)
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH)
            .unwrap();
        writer.persist_checkpoint().unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().modified().unwrap(),
            SystemTime::UNIX_EPOCH
        );
        writer
            .set_stage(BakeStage::Downsample, "dirty checkpoint")
            .unwrap();
        assert_ne!(
            fs::metadata(&path).unwrap().modified().unwrap(),
            SystemTime::UNIX_EPOCH
        );
        drop(job);
        fs::remove_dir_all(root.path()).unwrap();
    }

    #[test]
    fn resumed_checkpoint_verifies_one_payload_per_unit_and_regenerates_sorted_holes() {
        let root = temp_root("bounded-checkpoint-verification");
        let descriptor = descriptor(&root);
        let plan = BakePlan {
            planned_units: 3,
            stage: BakeStage::BaseTerrain,
        };
        let (request, mut writer) =
            fixture_writer(&root, descriptor.clone(), ArtifactKind::Images, plan);
        let keys = ResumeAdoptProducer::keys();
        let payloads = [fixture_png(1), fixture_png(2), fixture_png(3)];
        for (key, payload) in keys.into_iter().zip(&payloads) {
            writer
                .write_image_tile(UnitKey::Terrain { tile: key }, payload)
                .unwrap();
        }
        writer.persist_checkpoint().unwrap();
        drop(writer);
        fs::write(
            request.partial_dir.join(keys[1].relative_path().unwrap()),
            b"damaged",
        )
        .unwrap();
        let census = complete_census(&root);
        let usage = CacheUsage::for_root(
            &census,
            ArtifactRootId {
                revision: descriptor.revision(),
                kind: ArtifactKind::Images,
                key: descriptor.image_key().unwrap(),
                partial: true,
            },
        );
        let mut resumed = BakeWriter::open_at(
            &root,
            Arc::clone(&request.partial_dir),
            &request,
            plan,
            Arc::new(AtomicBool::new(false)),
            Arc::new(|_| {}),
            usage,
        )
        .unwrap();
        let payload_pointer = resumed.verification.as_ref().unwrap().payload.as_ptr();
        for next in 1..=2 {
            assert!(resumed.verify_next_checkpoint_unit().unwrap());
            let verification = resumed.verification.as_ref().unwrap();
            assert_eq!(verification.next, next);
            assert_eq!(verification.payload.as_ptr(), payload_pointer);
        }
        assert!(resumed.verify_next_checkpoint_unit().unwrap());
        assert!(resumed.verification.is_none());
        assert!(!resumed.verify_next_checkpoint_unit().unwrap());
        assert_eq!(resumed.checkpoint.completed.as_slice().len(), 2);
        assert!(resumed
            .completed_receipt(UnitKey::Terrain { tile: keys[1] })
            .is_none());
        assert_eq!(
            resumed.bytes,
            (payloads[0].len() + payloads[2].len()) as u64
        );
        assert!(!request
            .partial_dir
            .join(keys[1].relative_path().unwrap())
            .exists());
        let verified_census = complete_census(&root);
        assert_eq!(resumed.cache_bytes, verified_census.bytes);
        assert_eq!(resumed.cache_entries, verified_census.entries);
        assert_eq!(
            resumed.cache_product_entries,
            verified_census.product_entries
        );
        assert!(resumed
            .publish_unit(UnitKey::Terrain { tile: keys[1] }, &payloads[1])
            .unwrap());
        assert_eq!(
            resumed
                .checkpoint
                .completed
                .as_slice()
                .iter()
                .map(|unit| unit.key)
                .collect::<Vec<_>>(),
            keys.map(|tile| UnitKey::Terrain { tile }),
        );
        drop(resumed);
        fs::remove_dir_all(root.path()).unwrap();
    }

    fn finish_prune(cursor: &mut PruneCursor) {
        for _ in 0..MAX_ARTIFACT_ROOT_ENTRIES.saturating_mul(3) {
            match cursor.advance().unwrap() {
                PruneAdvance::Pending => {}
                PruneAdvance::Complete => return,
                PruneAdvance::Stale => panic!("prune cursor became stale"),
            }
        }
        panic!("prune cursor did not complete within the artifact-root bound");
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
        wait_for_status(&first, MapJobStatus::Paused);
        assert_eq!(producer.rasterized.load(AtomicOrdering::Relaxed), 1);
        let resumed = manager
            .retry(descriptor.clone(), MapDemand::Images)
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
            wait_ready(&reopened);
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

    #[test]
    fn image_bake_interrupt_resumes_only_remaining_units() {
        let root = temp_root("unit-interrupt-resume");
        let descriptor = descriptor(&root);
        let keys = ResumeAdoptProducer::keys();
        let producer = ResumeAdoptProducer::new(true);
        let manager = MapDemandManager::new(root.clone(), producer.clone());
        let first = manager
            .request(descriptor.clone(), MapDemand::Images)
            .unwrap();
        wait_for_status(&first, MapJobStatus::Paused);
        assert_eq!(producer.rasterized.load(AtomicOrdering::Relaxed), 1);
        let partial = root.partial_dir(
            descriptor.revision(),
            ArtifactKind::Images,
            descriptor.image_key().unwrap(),
        );
        let checkpoint = Checkpoint::decode(
            &fs::read(partial.join("checkpoint.json")).unwrap(),
            ArtifactIdentity::Image(descriptor.image_identity()),
        )
        .unwrap();
        assert_eq!(
            checkpoint.completed.as_slice()[0].key,
            UnitKey::Terrain { tile: keys[0] }
        );
        let resumed = manager.retry(descriptor, MapDemand::Images).unwrap();
        wait_ready(&resumed);
        assert_eq!(
            producer.rasterized.load(AtomicOrdering::Relaxed),
            3,
            "resume bakes only the two units after the checkpointed first tile"
        );
        let _ = fs::remove_dir_all(root.path());
    }

    /// Identity of the file behind `path`: a rewrite (temp file + rename)
    /// replaces it even when the bytes are identical.
    #[cfg(unix)]
    fn file_id(path: &Path) -> u64 {
        std::os::unix::fs::MetadataExt::ino(&fs::metadata(path).unwrap())
    }

    #[cfg(not(unix))]
    fn file_id(_path: &Path) -> u64 {
        0
    }

    #[cfg(unix)]
    #[test]
    fn finished_image_bake_publishes_the_raster_tiles_without_rewriting_them() {
        let root = temp_root("record-not-rewrite");
        let descriptor = descriptor(&root);
        let producer = ResumeAdoptProducer::new(false);
        let manager = MapDemandManager::new(root.clone(), producer.clone());
        let handle = manager
            .request(descriptor.clone(), MapDemand::Images)
            .unwrap();
        wait_ready(&handle);
        let images = handle.ready().unwrap().images.expect("images ready");
        let ready_dir = root.image_dir(descriptor.image_identity()).unwrap();
        let written = producer.written.lock().clone();
        assert_eq!(written.len(), 3);
        let mut buffer = Vec::new();
        for (key, raster_file) in written {
            // The published tile is the very file the raster wrote and synced.
            assert_eq!(
                file_id(&ready_dir.join(key.relative_path().unwrap())),
                raster_file,
                "finish rewrote tile {key:?}"
            );
            assert!(images.read_tile_into(key, &mut buffer).unwrap().is_some());
            let shared = images
                .read_tile_arc(key)
                .unwrap()
                .expect("listed tile returns shared bytes");
            assert_eq!(&*shared, &buffer);
        }
        drop(images);
        drop(handle);
        let _ = fs::remove_dir_all(root.path());
    }

    #[cfg(not(windows))]
    #[test]
    fn image_checkpoint_syncs_each_touched_directory_and_created_parent_once() {
        let root = temp_root("batch-directory-sync");
        let descriptor = descriptor(&root);
        let request = bake_request(&root, descriptor, ArtifactKind::Images).unwrap();
        let mut writer = BakeWriter::open(
            root.clone(),
            &request,
            BakePlan {
                planned_units: 64,
                stage: BakeStage::BaseTerrain,
            },
            Arc::new(AtomicBool::new(false)),
            Arc::new(|_| {}),
        )
        .unwrap();
        let partial = writer.directory().to_path_buf();
        let png = fixture_png(1);
        TEST_DIRECTORY_SYNCS.with(|trace| *trace.borrow_mut() = Some(Vec::new()));
        for x in 0..16 {
            for plane in 0..MAP_PLANES {
                writer
                    .write_image_tile(
                        UnitKey::Terrain {
                            tile: TileKey {
                                plane,
                                lod: (x / 6) as u8,
                                x,
                                z: 0,
                            },
                        },
                        &png,
                    )
                    .unwrap();
            }
        }
        writer.persist_checkpoint().unwrap();
        let synced = TEST_DIRECTORY_SYNCS.with(|trace| trace.borrow_mut().take().unwrap());
        let terrain = partial.join("terrain");
        let mut expected = Vec::new();
        for plane in 0..MAP_PLANES {
            let plane_dir = terrain.join(format!("p{plane}"));
            for lod in 0..3 {
                expected.push(plane_dir.join(format!("l{lod}")));
            }
            expected.push(plane_dir);
        }
        expected.push(terrain);
        // The checkpoint parent sync also commits the new terrain directory.
        expected.push(partial);
        assert_eq!(synced, expected);
        drop(writer);
        fs::remove_dir_all(root.path()).unwrap();
    }

    #[test]
    fn writer_rejects_missing_adopted_tile_and_checkpoints_its_own_write() {
        let root = temp_root("record-missing");
        let descriptor = descriptor(&root);
        let request = bake_request(&root, descriptor.clone(), ArtifactKind::Images).unwrap();
        let progress: Arc<dyn Fn(MapProgress) + Send + Sync> = Arc::new(|_| {});
        let mut writer = BakeWriter::open(
            root.clone(),
            &request,
            BakePlan {
                planned_units: 1,
                stage: BakeStage::BaseTerrain,
            },
            Arc::new(AtomicBool::new(false)),
            progress,
        )
        .unwrap();
        let key = ResumeAdoptProducer::keys()[0];
        let unit = UnitKey::Terrain { tile: key };
        let png = fixture_png(1);
        let expected = PayloadReceipt {
            bytes: png.len() as u32,
            sha256: Digest::of(&png),
        };
        assert!(writer.record_unit(unit, expected).is_err());
        assert!(writer.checkpoint().completed.as_slice().is_empty());
        let receipt = writer.write_image_tile(unit, &png).unwrap();
        assert_eq!(receipt, expected);
        assert_eq!(writer.completed_receipt(unit), Some(receipt));
        drop(writer);
        let _ = fs::remove_dir_all(root.path());
    }

    #[test]
    fn writer_caps_cumulative_image_bytes() {
        let root = temp_root("image-byte-cap");
        let descriptor = descriptor(&root);
        let request = bake_request(&root, descriptor.clone(), ArtifactKind::Images).unwrap();
        let mut writer = BakeWriter::open(
            root.clone(),
            &request,
            BakePlan {
                planned_units: 2,
                stage: BakeStage::BaseTerrain,
            },
            Arc::new(AtomicBool::new(false)),
            Arc::new(|_| {}),
        )
        .unwrap();
        let first = ResumeAdoptProducer::keys()[0];
        let second = ResumeAdoptProducer::keys()[1];
        let png = fixture_png(1);
        writer.bytes = max_artifact_bytes(ArtifactKind::Images) - png.len() as u64;
        let first_unit = UnitKey::Terrain { tile: first };
        let receipt = writer.write_image_tile(first_unit, &png).unwrap();
        assert_eq!(writer.bytes, max_artifact_bytes(ArtifactKind::Images));
        assert_eq!(writer.completed_receipt(first_unit), Some(receipt));
        let second_unit = UnitKey::Terrain { tile: second };
        assert!(matches!(
            writer.write_image_tile(second_unit, &png),
            Err(MapCacheError::Map(MapError::Limit("artifact bytes")))
        ));
        assert!(!writer
            .directory()
            .join(second.relative_path().unwrap())
            .exists());
        drop(writer);
        let _ = fs::remove_dir_all(root.path());
    }

    fn native_descriptor(jag: &Path, snapshot: &Path) -> MapProfileDescriptor {
        let content = Digest([9; 32]);
        let input = PreparedMapInput::with_snapshot(
            289,
            content,
            jag.to_path_buf(),
            snapshot.to_path_buf(),
        );
        let (image_policy, catalogue_policy) =
            crate::map_producer::map_artifact_policies().unwrap();
        MapProfileDescriptor::new(289, content, image_policy, catalogue_policy)
            .unwrap()
            .with_input(input)
            .unwrap()
    }

    #[test]
    fn native_image_resume_adopts_intact_tiles_and_rebakes_damaged_ones() {
        let root = temp_root("native-resume");
        let cold_root = temp_root("native-cold");
        let client_root = temp_root("native-resume-client");
        let (jag, snapshot) = synthetic_client_snapshot(client_root.path());
        let descriptor = native_descriptor(&jag, &snapshot);
        let cold_descriptor = native_descriptor(&jag, &snapshot);
        let cold_manager = MapDemandManager::new(
            cold_root.clone(),
            Arc::new(crate::map_producer::NativeMapProducer::new()),
        );
        let cold_identity = cold_descriptor.image_identity();
        let cold = cold_manager
            .request(cold_descriptor, MapDemand::Images)
            .unwrap();
        wait_ready(&cold);
        let cold_map = cold.ready().unwrap();
        let cold_images = cold_map.images.as_ref().unwrap();
        let tiles: Vec<_> = cold_images
            .manifest()
            .tiles
            .as_slice()
            .iter()
            .take(3)
            .copied()
            .collect();
        assert_eq!(tiles.len(), 3);
        let partial = root.partial_dir(
            descriptor.revision(),
            ArtifactKind::Images,
            descriptor.image_key().unwrap(),
        );
        for (index, tile) in tiles.iter().enumerate() {
            let bytes = cold_images
                .read_tile_arc(tile.key)
                .unwrap()
                .expect("cold manifest tile");
            let path = partial.join(tile.key.relative_path().unwrap());
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let mut bytes = bytes.to_vec();
            match index {
                1 => bytes.truncate(bytes.len() / 2),
                2 => {
                    let middle = bytes.len() / 2;
                    bytes[middle] ^= 0x5a;
                }
                _ => {}
            }
            fs::write(path, bytes).unwrap();
        }
        let intact = file_id(&partial.join(tiles[0].key.relative_path().unwrap()));
        let manager = MapDemandManager::new(
            root.clone(),
            Arc::new(crate::map_producer::NativeMapProducer::new()),
        );
        let resumed = manager
            .request(descriptor.clone(), MapDemand::Images)
            .unwrap();
        wait_ready(&resumed);
        let resumed_map = resumed.ready().unwrap();
        let resumed_images = resumed_map.images.as_ref().unwrap();
        let ready_dir = root.image_dir(descriptor.image_identity()).unwrap();
        assert_eq!(
            file_id(&ready_dir.join(tiles[0].key.relative_path().unwrap())),
            intact,
            "a valid uncheckpointed PNG is adopted without replacement"
        );
        for tile in &tiles {
            assert_eq!(
                resumed_images.read_tile_arc(tile.key).unwrap(),
                cold_images.read_tile_arc(tile.key).unwrap(),
                "damaged partial tiles are regenerated byte-identically"
            );
        }
        assert_eq!(
            fs::read(ready_dir.join("manifest.json")).unwrap(),
            fs::read(
                cold_root
                    .image_dir(cold_identity)
                    .unwrap()
                    .join("manifest.json")
            )
            .unwrap(),
            "resumed publication is byte-identical to a cold native bake"
        );
    }

    #[test]
    fn native_image_resume_rebakes_a_checkpointed_tile_whose_bytes_changed() {
        let root = temp_root("native-receipt");
        let cold_root = temp_root("native-receipt-cold");
        let client_root = temp_root("native-receipt-client");
        let (jag, snapshot) = synthetic_client_snapshot(client_root.path());
        let descriptor = native_descriptor(&jag, &snapshot);
        let cold_descriptor = native_descriptor(&jag, &snapshot);
        let cold_manager = MapDemandManager::new(
            cold_root.clone(),
            Arc::new(crate::map_producer::NativeMapProducer::new()),
        );
        let cold = cold_manager
            .request(cold_descriptor.clone(), MapDemand::Images)
            .unwrap();
        wait_ready(&cold);
        let cold_map = cold.ready().unwrap();
        let cold_images = cold_map.images.as_ref().unwrap().clone();
        let tile = cold_images.manifest().tiles.as_slice()[0];
        let request = bake_request(&root, descriptor.clone(), ArtifactKind::Images).unwrap();
        let plan = BakePlan {
            planned_units: cold_images.manifest().tiles.as_slice().len() as u32,
            stage: BakeStage::BaseTerrain,
        };
        let mut writer = BakeWriter::open(
            root.clone(),
            &request,
            plan,
            Arc::new(AtomicBool::new(false)),
            Arc::new(|_| {}),
        )
        .unwrap();
        let bytes = cold_images
            .read_tile_arc(tile.key)
            .unwrap()
            .expect("cold manifest tile");
        writer
            .write_image_tile(UnitKey::Terrain { tile: tile.key }, bytes.as_ref())
            .unwrap();
        let path = writer.directory().join(tile.key.relative_path().unwrap());
        assert_eq!(
            writer.completed_receipt(UnitKey::Terrain { tile: tile.key }),
            Some(tile.payload)
        );
        writer.persist_checkpoint().unwrap();
        fs::write(&path, fixture_png(9)).unwrap();
        drop(writer);

        let manager = MapDemandManager::new(
            root.clone(),
            Arc::new(crate::map_producer::NativeMapProducer::new()),
        );
        let resumed = manager
            .request(descriptor.clone(), MapDemand::Images)
            .unwrap();
        wait_ready(&resumed);
        let resumed_map = resumed.ready().unwrap();
        let resumed_images = resumed_map.images.as_ref().unwrap();
        assert_eq!(
            resumed_images.read_tile_arc(tile.key).unwrap(),
            cold_images.read_tile_arc(tile.key).unwrap(),
            "a checkpoint receipt must not bless bytes changed after checkpointing"
        );
        assert_eq!(
            fs::read(
                root.image_dir(descriptor.image_identity())
                    .unwrap()
                    .join("manifest.json")
            )
            .unwrap(),
            fs::read(
                cold_root
                    .image_dir(cold_descriptor.image_identity())
                    .unwrap()
                    .join("manifest.json")
            )
            .unwrap()
        );
    }

    #[test]
    fn crashed_image_checkpoint_window_rebakes_missing_and_partial_tiles() {
        use client::io::cache_289::synthetic_jag;

        const ACKNOWLEDGED_TILES: usize = 64;
        const UNACKNOWLEDGED_TILES: usize = 63;
        let root = temp_root("crashed-image-window");
        let cold_root = temp_root("crashed-image-reference");
        let client_root = temp_root("crashed-image-client");
        let (jag, snapshot) = synthetic_client_snapshot(client_root.path());
        let mut index = Vec::new();
        for x in 50u16..178 {
            index.extend((x << 8 | 50).to_be_bytes());
            index.extend(0u16.to_be_bytes());
            index.extend(u16::MAX.to_be_bytes());
            index.push(0);
        }
        fs::write(
            jag.join("versionlist"),
            synthetic_jag(&[("map_index", index.as_slice())]),
        )
        .unwrap();
        let descriptor = native_descriptor(&jag, &snapshot);
        let cold_manager = MapDemandManager::new(
            cold_root.clone(),
            Arc::new(crate::map_producer::NativeMapProducer::new()),
        );
        let cold = cold_manager
            .request(descriptor.clone(), MapDemand::Images)
            .unwrap();
        wait_ready(&cold);
        let reference = cold.ready().unwrap();
        let reference_images = reference.images.as_ref().unwrap();
        let tiles = reference_images.manifest().tiles.as_slice();
        assert!(tiles.len() > ACKNOWLEDGED_TILES + UNACKNOWLEDGED_TILES);

        let request = bake_request(&root, descriptor.clone(), ArtifactKind::Images).unwrap();
        let mut writer = BakeWriter::open(
            root.clone(),
            &request,
            BakePlan {
                planned_units: tiles.len() as u32,
                stage: BakeStage::BaseTerrain,
            },
            Arc::new(AtomicBool::new(false)),
            Arc::new(|_| {}),
        )
        .unwrap();
        let partial = writer.directory().to_path_buf();
        #[cfg(not(windows))]
        TEST_DIRECTORY_SYNCS.with(|trace| *trace.borrow_mut() = Some(Vec::new()));
        for tile in &tiles[..ACKNOWLEDGED_TILES] {
            let png = reference_images.read_tile_arc(tile.key).unwrap().unwrap();
            writer
                .write_image_tile(UnitKey::Terrain { tile: tile.key }, &png)
                .unwrap();
        }
        #[cfg(not(windows))]
        {
            let synced = TEST_DIRECTORY_SYNCS.with(|trace| trace.borrow_mut().take().unwrap());
            assert_eq!(
                synced.iter().filter(|path| **path == partial).count(),
                1,
                "the checkpoint parent also commits the new terrain directory"
            );
        }
        let checkpoint_path = partial.join("checkpoint.json");
        let checkpoint_bytes = fs::read(&checkpoint_path).unwrap();
        let acknowledged = Checkpoint::decode(
            &checkpoint_bytes,
            ArtifactIdentity::Image(descriptor.image_identity()),
        )
        .unwrap();
        assert_eq!(acknowledged.completed.as_slice().len(), ACKNOWLEDGED_TILES);
        let window = &tiles[ACKNOWLEDGED_TILES..ACKNOWLEDGED_TILES + UNACKNOWLEDGED_TILES];
        for tile in window {
            let png = reference_images.read_tile_arc(tile.key).unwrap().unwrap();
            writer
                .write_image_tile(UnitKey::Terrain { tile: tile.key }, &png)
                .unwrap();
        }
        let before_crash = fs::read(&checkpoint_path).unwrap();
        assert_eq!(
            Checkpoint::decode(
                &before_crash,
                ArtifactIdentity::Image(descriptor.image_identity()),
            )
            .unwrap()
            .completed
            .as_slice()
            .len(),
            ACKNOWLEDGED_TILES,
            "the crash window must not acknowledge its unfinished batch"
        );
        assert_eq!(before_crash, checkpoint_bytes);
        drop(writer);
        assert_eq!(
            fs::read(&checkpoint_path).unwrap(),
            checkpoint_bytes,
            "a crash must not flush the in-memory window"
        );

        // Lose the unacknowledged directory entries and leave the next tile
        // half-written, as a crash can do before its receipt is committed.
        for tile in window {
            fs::remove_file(partial.join(tile.key.relative_path().unwrap())).unwrap();
        }
        let interrupted = tiles[ACKNOWLEDGED_TILES + UNACKNOWLEDGED_TILES];
        let png = reference_images
            .read_tile_arc(interrupted.key)
            .unwrap()
            .unwrap();
        let interrupted_path = partial.join(interrupted.key.relative_path().unwrap());
        fs::create_dir_all(interrupted_path.parent().unwrap()).unwrap();
        fs::write(&interrupted_path, &png[..png.len() / 2]).unwrap();
        let intact = file_id(&partial.join(tiles[0].key.relative_path().unwrap()));
        assert!(!root
            .image_dir(descriptor.image_identity())
            .unwrap()
            .exists());

        let manager = MapDemandManager::new(
            root.clone(),
            Arc::new(crate::map_producer::NativeMapProducer::new()),
        );
        let resumed = manager
            .request(descriptor.clone(), MapDemand::Images)
            .unwrap();
        wait_ready(&resumed);
        let ready = resumed.ready().unwrap();
        let images = ready.images.as_ref().unwrap();
        assert_eq!(
            images.manifest().encode().unwrap(),
            reference_images.manifest().encode().unwrap()
        );
        for tile in tiles {
            assert_eq!(
                images.read_tile_arc(tile.key).unwrap(),
                reference_images.read_tile_arc(tile.key).unwrap(),
                "resume must regenerate every missing or partial tile"
            );
        }
        assert_eq!(
            file_id(
                &root
                    .image_dir(descriptor.image_identity())
                    .unwrap()
                    .join(tiles[0].key.relative_path().unwrap())
            ),
            intact,
            "resume must preserve an acknowledged intact tile"
        );
    }

    #[test]
    fn closing_the_last_handle_drops_the_finished_job() {
        let root = temp_root("reap-drops-job");
        let manager = MapDemandManager::new(root.clone(), FixtureProducer::new(false));
        let handle = manager
            .request(descriptor(&root), MapDemand::Images)
            .unwrap();
        wait_ready(&handle);
        let job = Arc::downgrade(&handle.status);
        drop(handle);
        manager.reap();
        assert!(
            job.upgrade().is_none(),
            "a closed, finished job is still held by the manager"
        );
        let _ = fs::remove_dir_all(root.path());
    }

    #[test]
    fn repeated_image_requests_share_one_bake() {
        let root = temp_root("join");
        let producer = FixtureProducer::new(false);
        let manager = MapDemandManager::new(root.clone(), producer.clone());
        let descriptor = descriptor(&root);
        let first = manager
            .request(descriptor.clone(), MapDemand::Images)
            .unwrap();

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
    fn identity_switch_opens_new_profile_after_finished_demand() {
        let root = temp_root("switch");
        let producer = FixtureProducer::new(false);

        let manager = MapDemandManager::new(root.clone(), producer.clone());
        let first_descriptor = descriptor(&root);
        let first = manager
            .request(first_descriptor, MapDemand::Images)
            .unwrap();
        wait_ready(&first);
        drop(first);

        let second_descriptor =
            MapProfileDescriptor::new(274, Digest([4; 32]), Digest([5; 32]), Digest([6; 32]))
                .unwrap();
        let second = manager
            .request(second_descriptor.clone(), MapDemand::CatalogueOnly)
            .unwrap();

        wait_ready(&second);
        assert!(root
            .catalogue_dir(second_descriptor.catalogue_identity())
            .unwrap()
            .is_dir());
        drop(second);
        let _ = fs::remove_dir_all(root.path());
    }

    #[test]
    fn ready_artifact_checkpoint_is_removed_when_reopened() {
        let root = temp_root("checkpoint-boundary");
        let descriptor = descriptor(&root);
        let manager = MapDemandManager::new(root.clone(), FixtureProducer::new(false));
        let baked = manager
            .request(descriptor.clone(), MapDemand::CatalogueOnly)
            .unwrap();
        wait_ready(&baked);
        let ready = root.catalogue_dir(descriptor.catalogue_identity()).unwrap();
        drop(baked);

        fs::write(ready.join("checkpoint.json"), b"stale checkpoint").unwrap();
        let reopened_manager = MapDemandManager::new(root.clone(), FixtureProducer::new(false));
        let reopened = reopened_manager
            .request(descriptor, MapDemand::CatalogueOnly)
            .unwrap();
        wait_ready(&reopened);
        assert!(!ready.join("checkpoint.json").exists());
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

        let keys = FixtureProducer::tile_keys();
        let (_request, mut writer) = fixture_writer(
            &root,
            descriptor.clone(),
            ArtifactKind::Images,
            BakePlan {
                planned_units: keys.len() as u32,
                stage: BakeStage::BaseTerrain,
            },
        );
        let checkpoint_before = fs::read(writer.partial.join("checkpoint.json")).unwrap();
        let key = keys[0];
        let unit = UnitKey::Terrain { tile: key };
        let payload = fixture_png(1);
        assert!(writer.publish_unit(unit, &payload).unwrap());
        assert_eq!(
            checkpoint_before,
            fs::read(writer.partial.join("checkpoint.json")).unwrap()
        );
        writer.persist_checkpoint().unwrap();
        drop(writer);

        let resumed_producer = FixtureProducer::new(false);
        let resumed = MapDemandManager::new(root.clone(), resumed_producer.clone());
        let handle = resumed.request(descriptor, MapDemand::Images).unwrap();
        wait_ready(&handle);
        assert_eq!(resumed_producer.rasterized.load(AtomicOrdering::Relaxed), 1);
        let image_dir = root
            .image_dir(handle.descriptor().image_identity())
            .unwrap();
        assert!(!image_dir.join("checkpoint.json").exists());
    }

    #[test]
    fn client_input_requires_a_prepared_cache_lease() {
        let root = temp_root("client-input-missing");
        let request = bake_request(&root, descriptor(&root), ArtifactKind::Catalogue).unwrap();
        assert!(matches!(
            request.client_input(),
            Err(MapCacheError::NoPreparedCache)
        ));
    }

    #[test]
    fn bad_catalogue_receipt_is_rejected_before_ready_publication() {
        let root = temp_root("bad-receipt");
        let descriptor = descriptor(&root);
        let identity = descriptor.catalogue_identity();
        let (request, mut writer) = fixture_writer(
            &root,
            descriptor.clone(),
            ArtifactKind::Catalogue,
            BakePlan {
                planned_units: 1,
                stage: BakeStage::Catalogue,
            },
        );
        let pois = fixture_cursor(&request).catalogue().unwrap();
        let payload = encode_json_bounded(&pois).unwrap();
        let unit = UnitKey::ClientPois;
        let receipt = PayloadReceipt {
            bytes: payload.len() as u32,
            sha256: Digest::of(&payload),
        };
        assert!(writer.publish_unit(unit, &payload).unwrap());
        writer.persist_checkpoint().unwrap();
        let output = BakeOutput::Catalogue(CatalogueManifest {
            schema: CATALOGUE_SCHEMA,
            identity,
            key: identity.key().unwrap(),
            record_count: 1,
            payload: receipt,
        });
        match writer.finish(output) {
            Err(MapCacheError::Map(MapError::Invalid("POI receipt count"))) => {}
            Err(_) => panic!("bad catalogue receipt returned the wrong error"),
            Ok(_) => panic!("bad catalogue receipt was published"),
        }
        assert!(!root.catalogue_dir(identity).unwrap().exists());
    }

    #[test]
    fn pruning_skips_a_leased_lru_root_and_removes_an_unleased_root() {
        let root = temp_root("prune-leased-lru");
        let first = descriptor(&root);
        let owner = {
            let installer = MapDemandManager::new(root.clone(), FixtureProducer::new(false));
            let handle = installer.request(first.clone(), MapDemand::Images).unwrap();
            wait_ready(&handle);
            handle.ready().unwrap().images.unwrap()
        };
        let second =
            MapProfileDescriptor::new(274, Digest([4; 32]), Digest([2; 32]), Digest([3; 32]))
                .unwrap();
        {
            let installer = MapDemandManager::new(root.clone(), FixtureProducer::new(false));
            let handle = installer
                .request(second.clone(), MapDemand::Images)
                .unwrap();
            wait_ready(&handle);
        }
        let leased = root.image_dir(first.image_identity()).unwrap();
        let unleased = root.image_dir(second.image_identity()).unwrap();
        fs::File::create(unleased.join("unused-padding.bin"))
            .unwrap()
            .set_len(MAX_GENERATED_CACHE_BYTES + (16 << 20))
            .unwrap();
        for (directory, seconds) in [(&leased, 3600), (&unleased, 7200)] {
            let mut options = fs::OpenOptions::new();
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                options.access_mode(0x0100).custom_flags(0x0200_0000);
            }
            #[cfg(not(windows))]
            options.read(true);
            options
                .open(directory)
                .unwrap()
                .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds))
                .unwrap();
        }
        let producer = FixtureProducer::new(false);
        let manager = MapDemandManager::new(root.clone(), producer.clone());
        let (input, request) = manager
            .input_for(
                &first,
                JobKey {
                    catalogue: first.catalogue_key().unwrap(),
                    image: first.image_key().unwrap(),
                },
                1,
                ArtifactKind::Catalogue,
            )
            .unwrap();
        let mut job = MapBakeJob::new(
            input,
            root.clone(),
            producer,
            None,
            request,
            Arc::new(AtomicUsize::new(DEMAND_ADOPT_IMAGES)),
        )
        .unwrap();
        job.accept_census(complete_census(&root)).unwrap();
        let mut prune = job.prune_cursor.take().expect("the cache exceeds its cap");
        assert_eq!(
            prune.id,
            ArtifactRootId {
                revision: 274,
                kind: ArtifactKind::Images,
                key: second.image_key().unwrap(),
                partial: false,
            },
            "the least recently used leased root is not a prune candidate"
        );
        finish_prune(&mut prune);
        assert!(leased.is_dir(), "the admitted terrain owner remains valid");
        assert!(!unleased.exists(), "the unleased root supplies capacity");
        assert_eq!(owner.manifest().identity, first.image_identity());
    }

    #[test]
    fn marked_prune_blocks_open_and_writer_then_resumes_after_crash() {
        let root = temp_root("prune-crash-resume");
        let manager = MapDemandManager::new(root.clone(), FixtureProducer::new(false));
        let descriptor = descriptor(&root);
        let handle = manager
            .request(descriptor.clone(), MapDemand::Images)
            .unwrap();
        wait_ready(&handle);
        drop(handle);

        let image_dir = root.image_dir(descriptor.image_identity()).unwrap();
        let root_id = ArtifactRootId {
            revision: descriptor.revision(),
            kind: ArtifactKind::Images,
            key: descriptor.image_key().unwrap(),
            partial: false,
        };
        let marker_path = root.prune_marker_path(root_id);
        let lock_path = root.lock_path(root_id.revision, root_id.kind, root_id.key);
        let mut capacity = root.try_acquire_capacity().unwrap().unwrap();
        let leased = root
            .open_images(descriptor.image_identity(), &mut capacity)
            .unwrap()
            .unwrap();
        drop(capacity);

        let census = complete_census(&root);
        let summary = census
            .roots
            .iter()
            .find(|summary| summary.id == root_id)
            .unwrap();
        let mut prune = PruneCursor::new(root.clone(), summary, census.generation).unwrap();
        assert_eq!(prune.advance().unwrap(), PruneAdvance::Stale);
        assert!(
            !marker_path.exists(),
            "a live ready lease must block marking"
        );
        drop(leased);

        assert_eq!(prune.advance().unwrap(), PruneAdvance::Pending);
        assert!(marker_path.is_file());
        assert_eq!(prune.advance().unwrap(), PruneAdvance::Pending);
        assert!(image_dir.is_dir());
        assert!(!image_dir.join("manifest.json").exists());

        let census = complete_census(&root);
        let request = bake_request(&root, descriptor.clone(), ArtifactKind::Images).unwrap();
        let producer = FixtureProducer::new(false);
        let input = MapBakeInput {
            key: JobKey {
                catalogue: descriptor.catalogue_key().unwrap(),
                image: descriptor.image_key().unwrap(),
            },
            generation: 1,
            artifact: ArtifactKind::Images,
            charge: producer.charge(&request).unwrap(),
        };
        let mut job = MapBakeJob::new(
            input,
            root.clone(),
            producer,
            None,
            request,
            Arc::new(AtomicUsize::new(DEMAND_BAKE_IMAGES)),
        )
        .unwrap();
        job.census_result = Some(census);
        assert!(matches!(
            job.acquire_exclusive().unwrap(),
            ExclusiveAcquire::Busy
        ));
        let mut capacity = root.try_acquire_capacity().unwrap().unwrap();
        assert!(root
            .open_images(descriptor.image_identity(), &mut capacity)
            .unwrap()
            .is_none());
        drop(capacity);

        drop(prune);
        let census = complete_census(&root);
        let summary = census
            .roots
            .iter()
            .find(|summary| summary.id == root_id)
            .unwrap();
        let mut resumed = PruneCursor::new(root.clone(), summary, census.generation).unwrap();
        finish_prune(&mut resumed);
        assert!(!marker_path.exists());
        assert!(!image_dir.exists());
        assert!(lock_path.is_file());
    }

    #[test]
    fn foreign_prune_marker_is_preserved_and_never_authorizes_deletion() {
        let root = temp_root("foreign-prune-marker");
        let manager = MapDemandManager::new(root.clone(), FixtureProducer::new(false));
        let descriptor = descriptor(&root);
        let handle = manager
            .request(descriptor.clone(), MapDemand::Images)
            .unwrap();
        wait_ready(&handle);
        drop(handle);

        let image_dir = root.image_dir(descriptor.image_identity()).unwrap();
        let root_id = ArtifactRootId {
            revision: descriptor.revision(),
            kind: ArtifactKind::Images,
            key: descriptor.image_key().unwrap(),
            partial: false,
        };
        let census = complete_census(&root);
        let summary = census
            .roots
            .iter()
            .find(|summary| summary.id == root_id)
            .unwrap();
        let marker_path = root.prune_marker_path(root_id);
        let foreign_descriptor = MapProfileDescriptor::new(
            descriptor.revision(),
            Digest([91; 32]),
            Digest([92; 32]),
            Digest([93; 32]),
        )
        .unwrap();
        let foreign_marker = serde_json::to_vec(&PruneMarker {
            schema: PRUNE_MARKER_SCHEMA,
            identity: ArtifactIdentity::Image(foreign_descriptor.image_identity()),
            partial: false,
        })
        .unwrap();
        fs::write(&marker_path, &foreign_marker).unwrap();

        let mut prune = PruneCursor::new(root.clone(), summary, census.generation).unwrap();
        assert!(matches!(
            prune.advance(),
            Err(MapCacheError::Map(MapError::Path))
        ));
        assert_eq!(fs::read(&marker_path).unwrap(), foreign_marker);
        assert!(image_dir.is_dir());
        assert!(image_dir.join("manifest.json").is_file());
        let mut capacity = root.try_acquire_capacity().unwrap().unwrap();
        assert!(matches!(
            root.open_images(descriptor.image_identity(), &mut capacity),
            Err(MapCacheError::Map(MapError::Path))
        ));
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

    #[test]
    fn same_key_bake_across_processes_publishes_once() {
        const CHILD_ROOT: &str = "274BOT_MAP_CACHE_SAME_KEY_CHILD_ROOT";
        if let Some(path) = std::env::var_os(CHILD_ROOT) {
            let root = MapCacheRoot::from_existing_root(path);
            let manager = MapDemandManager::new(root.clone(), FixtureProducer::new(false));
            let handle = manager
                .request(descriptor(&root), MapDemand::Images)
                .unwrap();
            wait_ready(&handle);
            println!(
                "\nPUBLISHED_DIRECTORY_ID={}",
                file_id(&root.image_dir(descriptor(&root).image_identity()).unwrap())
            );
            return;
        }

        let root = temp_root("same-key-process");
        let children: Vec<_> = (0..2)
            .map(|_| {
                spawn_test_child(
                    "map_cache::tests::same_key_bake_across_processes_publishes_once",
                    (CHILD_ROOT, root.path()),
                )
            })
            .collect();
        let outputs: Vec<_> = children
            .into_iter()
            .map(|child| child.wait_with_output().unwrap())
            .collect();
        let mut directory_id = None;
        for output in outputs {
            assert!(
                output.status.success(),
                "same-key child failed: stdout={} stderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            let observed = String::from_utf8_lossy(&output.stdout)
                .lines()
                .find_map(|line| line.strip_prefix("PUBLISHED_DIRECTORY_ID="))
                .unwrap()
                .parse::<u64>()
                .unwrap();
            if let Some(first) = directory_id {
                assert_eq!(observed, first, "the published cache must not be replaced");
            } else {
                directory_id = Some(observed);
            }
        }
        let descriptor = descriptor(&root);
        let ready = root.image_dir(descriptor.image_identity()).unwrap();
        for (index, tile) in FixtureProducer::tile_keys().into_iter().enumerate() {
            assert_eq!(
                fs::read(ready.join(tile.relative_path().unwrap())).unwrap(),
                fixture_png(index as u8 + 1)
            );
        }
        assert!(!root
            .partial_dir(
                descriptor.revision(),
                ArtifactKind::Images,
                descriptor.image_key().unwrap(),
            )
            .exists());
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

    #[test]
    fn catalogue_only_request_reuses_ready_catalogue_in_other_manager() {
        let root = temp_root("tui-manager-after-panel-bake");
        let panel = MapDemandManager::new(root.clone(), FixtureProducer::new(false));
        let panel_handle = panel.request(descriptor(&root), MapDemand::Images).unwrap();
        wait_ready(&panel_handle);

        let tui_producer = FixtureProducer::new(false);
        let tui = MapDemandManager::new(root.clone(), tui_producer.clone());
        let tui_handle = tui
            .request(descriptor(&root), MapDemand::CatalogueOnly)
            .unwrap();
        wait_ready(&tui_handle);
        assert!(tui_handle.ready().unwrap().images.is_none());
        assert_eq!(tui_producer.runs.load(AtomicOrdering::Relaxed), 0);

        let catalogue_lock = root.lock_path(
            274,
            ArtifactKind::Catalogue,
            descriptor(&root).catalogue_key().unwrap(),
        );
        assert!(PublishLock::try_acquire_shared(&catalogue_lock)
            .unwrap()
            .is_some());
        assert!(PublishLock::try_acquire(&catalogue_lock).unwrap().is_none());
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
    fn catalogue_only_child_reuses_ready_catalogue_from_panel_bake() {
        let root = temp_root("tui-process-after-panel-bake");
        let panel = MapDemandManager::new(root.clone(), FixtureProducer::new(false));
        let panel_handle = panel.request(descriptor(&root), MapDemand::Images).unwrap();
        wait_ready(&panel_handle);
        let child = spawn_test_child(
            "map_cache::tests::catalogue_only_child",
            (TUI_CHILD_ENV, root.path()),
        );
        assert_children_succeeded(vec![child]);
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
        wait_ready(&other_handle);
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

    /// A filesystem refusal is handled only on a later admitted map turn.
    #[test]
    fn filesystem_refusal_is_not_retried_inside_a_worker_dispatch() {
        let mut calls = 0;
        let result: io::Result<()> = run_once(|| {
            calls += 1;
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        });
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(calls, 1);
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
        let handle = manager
            .request(descriptor, MapDemand::CatalogueOnly)
            .unwrap();
        wait_for_status(
            &handle,
            MapJobStatus::Failed(MapCacheError::Map(MapError::Path).to_string()),
        );
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
        let lock = PublishLock::try_acquire(&lock_path).unwrap().unwrap();
        assert!(PublishLock::try_acquire(&lock_path).unwrap().is_none());
        drop(lock);
        let reacquired = PublishLock::try_acquire(&lock_path).unwrap().unwrap();
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
        let _lock = PublishLock::try_acquire(&lock_path).unwrap().unwrap();
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
                start.elapsed() < Duration::from_secs(60),
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

    #[test]
    fn census_transfers_its_only_summary_allocation() {
        let root = temp_root("census-summary-transfer");
        let mut cursor = CacheCensusCursor::new(root.clone());
        let allocation = cursor.roots.as_ptr();
        let result = loop {
            if let Some(result) = cursor.advance().unwrap() {
                break result;
            }
            assert_eq!(cursor.roots.as_ptr(), allocation);
        };
        assert_eq!(result.roots.as_ptr(), allocation);
        assert_eq!(result.roots.capacity(), MAX_CACHE_ARTIFACT_ROOTS);
        assert_eq!(cursor.roots.capacity(), 0);
        drop(result);
        drop(cursor);
        fs::remove_dir_all(root.path()).unwrap();
    }

    const FULL_CENSUS_OPT_IN: &str = "274BOT_MAP_CACHE_FULL_CENSUS";

    fn census_reservation_bytes() -> usize {
        [
            (std::mem::size_of::<u16>(), MAX_CACHE_ARTIFACT_ROOTS),
            (
                std::mem::size_of::<(u16, ArtifactKind)>(),
                2 * MAX_CACHE_ARTIFACT_ROOTS,
            ),
            (
                std::mem::size_of::<KindDirectoryEntry>(),
                MAX_ARTIFACT_ROOT_ENTRIES,
            ),
            (
                std::mem::size_of::<ArtifactRootSummary>(),
                MAX_CACHE_ARTIFACT_ROOTS,
            ),
            (
                std::mem::size_of::<CensusDirectory>(),
                2 * MAX_ARTIFACT_ROOT_ENTRIES,
            ),
            (std::mem::size_of::<usize>(), 4 * MAX_ARTIFACT_ROOT_ENTRIES),
            (
                std::mem::size_of::<OsString>(),
                2 * MAX_ARTIFACT_ROOT_ENTRIES,
            ),
            (
                1,
                MAX_CENSUS_NAME_BYTES * MAX_ARTIFACT_ROOT_ENTRIES + 2 * MAX_CENSUS_PATH_BYTES,
            ),
        ]
        .into_iter()
        .map(|(size, count)| size * count)
        .sum()
    }

    fn census_retained_bytes(cursor: &CacheCensusCursor) -> usize {
        let mut bytes = cursor.revisions.capacity() * std::mem::size_of::<u16>()
            + cursor.kind_directories.capacity() * std::mem::size_of::<(u16, ArtifactKind)>()
            + cursor.kind_entries.capacity() * std::mem::size_of::<KindDirectoryEntry>()
            + cursor.roots.capacity() * std::mem::size_of::<ArtifactRootSummary>();
        if let Some(tree) = &cursor.tree {
            bytes += tree.directories.capacity() * std::mem::size_of::<CensusDirectory>();
            bytes += tree.pending.capacity() * std::mem::size_of::<usize>();
            bytes += tree
                .directories
                .iter()
                .map(|directory| directory.name.capacity())
                .sum::<usize>();
        }
        bytes
    }

    #[test]
    #[ignore = "full-cap measurement; run with 274BOT_MAP_CACHE_FULL_CENSUS=1 and --ignored"]
    fn full_cap_cache_census_measurement() {
        assert_eq!(
            std::env::var(FULL_CENSUS_OPT_IN).ok().as_deref(),
            Some("1"),
            "opt in explicitly with {FULL_CENSUS_OPT_IN}=1"
        );

        let root = temp_root("full-cap-census");
        let root_path = root.path();
        let setup_start = Instant::now();
        let revision_dir = root.revision_dir(289);
        let kind_dir = root.kind_dir(289, ArtifactKind::Catalogue);
        fs::create_dir_all(&kind_dir).unwrap();
        const ROOTS: usize = MAX_CACHE_ARTIFACT_ROOTS;
        const FULL_ROOT_ENTRIES: usize = MAX_ARTIFACT_ROOT_ENTRIES;
        const REDUCED_ROOTS: usize = 4;
        let policy = Digest([0x51; 32]);
        for index in 0..ROOTS {
            let content = Digest::of(&(index as u64).to_be_bytes());
            let identity = CatalogueIdentity {
                revision: 289,
                content,
                policy,
            };
            let artifact_root = root.catalogue_dir(identity).unwrap();
            fs::create_dir(&artifact_root).unwrap();

            let payload = ClientPois {
                schema: CATALOGUE_SCHEMA,
                identity,
                coverage: Coverage {
                    npc_placements: CoverageLevel::Unavailable,
                    bank_services: CoverageLevel::Limited,
                    place_labels: CoverageLevel::Unavailable,
                    unresolved: nav::map::Rows::new(Vec::new()).unwrap(),
                },
                records: nav::map::Rows::new(Vec::new()).unwrap(),
            }
            .encode()
            .unwrap();
            let manifest = CatalogueManifest {
                schema: CATALOGUE_SCHEMA,
                identity,
                key: identity.key().unwrap(),
                record_count: 0,
                payload: PayloadReceipt {
                    bytes: payload.len() as u32,
                    sha256: Digest::of(&payload),
                },
            };
            let manifest = serde_json::to_vec(&manifest).unwrap();
            fs::write(artifact_root.join("client-pois.json"), &payload).unwrap();
            fs::write(artifact_root.join("manifest.json"), &manifest).unwrap();

            // Each cache-root summary includes its root directory and all
            // descendant entries. Four roots are one entry short so the
            // cache's four directory/control entries land exactly at the
            // global cap: cache root, capacity lock, revision, kind.
            let files_in_root = FULL_ROOT_ENTRIES - 1 - if index < REDUCED_ROOTS { 1 } else { 0 };
            let links = files_in_root - 3;
            let target = artifact_root.join("payload-0000");
            fs::write(&target, []).unwrap();
            for link in 1..=links {
                fs::hard_link(&target, artifact_root.join(format!("payload-{link:04}"))).unwrap();
            }
        }
        let setup_elapsed = setup_start.elapsed();
        assert!(revision_dir.is_dir());
        assert!(kind_dir.is_dir());

        let mut cursor = CacheCensusCursor::new(root.clone());
        let scan_start = Instant::now();
        let mut advance_count = 0usize;
        let mut advance_times = Vec::with_capacity(ROOTS + 8);
        let mut cursor_observed_peak = census_retained_bytes(&cursor);
        let result = loop {
            advance_count += 1;
            let advance_start = Instant::now();
            let result = cursor.advance().unwrap();
            advance_times.push(advance_start.elapsed());
            cursor_observed_peak = cursor_observed_peak.max(census_retained_bytes(&cursor));
            if let Some(result) = result {
                break result;
            }
            assert!(
                advance_count <= ROOTS + 8,
                "census did not complete within bounded steps"
            );
        };
        let scan_elapsed = scan_start.elapsed();

        let expected_entries = MAX_CACHE_SCAN_ENTRIES;
        let expected_product_entries = expected_entries - 4;
        assert_eq!(result.entries, expected_entries);
        assert_eq!(result.product_entries, expected_product_entries);
        assert!(result.bytes <= MAX_GENERATED_CACHE_BYTES);
        assert_eq!(result.overrun, None);
        assert!(!result.foreign_tree);
        const SUMMARY_BOUND_FROM_256_MIB: usize = (256 * 1024 * 1024) / 260_457;
        assert_eq!(MAX_CACHE_ARTIFACT_ROOTS, SUMMARY_BOUND_FROM_256_MIB);
        assert_eq!(result.roots.len(), ROOTS);
        assert!(result.roots.len() <= SUMMARY_BOUND_FROM_256_MIB);
        assert_eq!(result.roots.capacity(), MAX_CACHE_ARTIFACT_ROOTS);
        assert!(result.roots.iter().all(|summary| {
            summary.identity.is_some()
                && summary.fully_censused
                && summary.safe_to_prune
                && summary.entries <= MAX_ARTIFACT_ROOT_ENTRIES
        }));
        assert_eq!(
            result
                .roots
                .iter()
                .filter(|summary| summary.entries == FULL_ROOT_ENTRIES)
                .count(),
            ROOTS - REDUCED_ROOTS
        );
        assert_eq!(
            result
                .roots
                .iter()
                .filter(|summary| summary.entries == FULL_ROOT_ENTRIES - 1)
                .count(),
            REDUCED_ROOTS
        );
        let result_summary_bytes =
            result.roots.capacity() * std::mem::size_of::<ArtifactRootSummary>();
        assert!(
            result_summary_bytes
                <= MAX_CACHE_ARTIFACT_ROOTS * std::mem::size_of::<ArtifactRootSummary>(),
            "summary capacity exceeded the configured root-summary bound"
        );
        assert_eq!(
            cursor.roots.capacity(),
            0,
            "the completed cursor must transfer its sole summary allocation"
        );
        let scan_summary_bytes = result_summary_bytes
            + cursor.roots.capacity() * std::mem::size_of::<ArtifactRootSummary>();
        let production_traversal_reservation = census_reservation_bytes();
        assert!(cursor_observed_peak <= production_traversal_reservation);
        assert_eq!(scan_summary_bytes, result_summary_bytes);
        advance_times.sort_unstable();
        let advance_p50 = advance_times[(advance_times.len() - 1) * 50 / 100];
        let advance_p99 = advance_times[(advance_times.len() - 1) * 99 / 100];
        let advance_max = advance_times[advance_times.len() - 1];
        eprintln!(
            "map-cache full-cap advance: p50_us={} p99_us={} max_us={} observations={}",
            advance_p50.as_micros(),
            advance_p99.as_micros(),
            advance_max.as_micros(),
            advance_times.len(),
        );

        eprintln!(
            "map-cache full-cap census: setup_ms={} scan_ms={} advances={} entries={} product_entries={} bytes={} roots={}/{} result_summary_capacity={} result_summary_bytes={} result_plus_cursor_summary_bytes={} traversal_retained_after_scan_bytes={} cursor_observed_retained_peak_bytes={} traversal_retained_peak_reserved_bytes={}",
            setup_elapsed.as_millis(),
            scan_elapsed.as_millis(),
            advance_count,
            result.entries,
            result.product_entries,
            result.bytes,
            result.roots.len(),
            result.roots.capacity(),
            result.roots.capacity(),
            result_summary_bytes,
            scan_summary_bytes,
            census_retained_bytes(&cursor),
            cursor_observed_peak,
            production_traversal_reservation,
        );
        drop(result);
        drop(cursor);
        assert!(root_path.is_dir(), "fixture root unexpectedly disappeared");
        fs::remove_dir_all(root_path).unwrap();
    }

    #[test]
    fn cancellation_resumes_a_partly_checkpointed_native_image_batch() {
        use api::work::JobState;
        use client::io::cache_289::synthetic_jag;
        use std::sync::mpsc::{self, Receiver, Sender};

        const CHILD_ROOT: &str = "BOT_MAP_ACTIVE_BATCH_CHILD_ROOT";
        let Some(path) = std::env::var_os(CHILD_ROOT) else {
            let root = temp_root("active-image-batch");
            let child = spawn_test_child(
                "map_cache::tests::cancellation_resumes_a_partly_checkpointed_native_image_batch",
                (CHILD_ROOT, root.path()),
            );
            assert_children_succeeded(vec![child]);
            return;
        };

        struct BatchProducer {
            reached: Sender<()>,
            release: Mutex<Option<Receiver<()>>>,
        }
        struct BatchCursor {
            inner: Box<dyn MapBakeCursor>,
            reached: Option<Sender<()>>,
            release: Option<Receiver<()>>,
        }
        impl MapBakeProducer for BatchProducer {
            fn charge(&self, request: &BakeRequest) -> Result<WorkCharge, MapCacheError> {
                crate::map_producer::NativeMapProducer::new().charge(request)
            }

            fn cursor(
                &self,
                request: &BakeRequest,
                partial: Arc<PathBuf>,
            ) -> Result<Box<dyn MapBakeCursor>, MapCacheError> {
                let inner =
                    crate::map_producer::NativeMapProducer::new().cursor(request, partial)?;
                if request.artifact() != ArtifactKind::Images {
                    return Ok(inner);
                }
                Ok(Box::new(BatchCursor {
                    inner,
                    reached: Some(self.reached.clone()),
                    release: self.release.lock().take(),
                }))
            }
        }
        impl MapBakeCursor for BatchCursor {
            fn plan(&self) -> Option<BakePlan> {
                self.inner.plan()
            }

            fn step(
                &mut self,
                context: &mut StepContext<'_, MapBakePhase>,
                mut writer: Option<&mut BakeWriter>,
            ) -> Result<MapBakeCursorStep, MapCacheError> {
                let result = self.inner.step(context, writer.as_deref_mut())?;
                let at_pending_batch = matches!(result, MapBakeCursorStep::Yielded)
                    && writer.as_ref().is_some_and(|writer| {
                        writer.checkpoint.completed.as_slice().len() >= CHECKPOINT_BATCH_TILES
                            && writer.pending_image_tiles > 0
                    });
                if let Some(reached) = self.reached.take_if(|_| at_pending_batch) {
                    // Hold the actual request inside its native cursor.
                    reached.send(()).unwrap();
                    self.release
                        .take()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(10))
                        .unwrap();
                }
                Ok(result)
            }

            fn progress(&self) -> MapProgressSnapshot {
                self.inner.progress()
            }

            fn take_completed_units(&mut self) -> Vec<CompletedUnit> {
                self.inner.take_completed_units()
            }
        }

        let root = MapCacheRoot::from_existing_root(path);
        let cold_root = temp_root("active-batch-reference");
        let client_root = temp_root("active-batch-client");
        let (jag, snapshot) = synthetic_client_snapshot(client_root.path());
        // Reuse the real decoded land record across enough squares for a
        // complete checkpoint and an unfinished subsequent batch.
        let mut index = Vec::new();
        for x in 50..50 + CHECKPOINT_BATCH_TILES as u16 {
            index.extend((x << 8 | 50).to_be_bytes());
            index.extend(0u16.to_be_bytes());
            index.extend(u16::MAX.to_be_bytes());
            index.push(0);
        }
        fs::write(
            jag.join("versionlist"),
            synthetic_jag(&[("map_index", index.as_slice())]),
        )
        .unwrap();
        let descriptor = native_descriptor(&jag, &snapshot);
        let cold_manager = MapDemandManager::new(
            cold_root.clone(),
            Arc::new(crate::map_producer::NativeMapProducer::new()),
        );
        let cold = cold_manager
            .request(descriptor.clone(), MapDemand::Images)
            .unwrap();
        wait_ready(&cold);
        let reference = cold.ready().unwrap();
        let reference_images = reference.images.as_ref().unwrap();
        assert!(reference_images.manifest().tiles.as_slice().len() > CHECKPOINT_BATCH_TILES);

        let (batch_reached, batch_wait) = mpsc::channel();
        let (gate_release, gate_continue) = mpsc::channel();
        let manager = MapDemandManager::new(
            root.clone(),
            Arc::new(BatchProducer {
                reached: batch_reached,
                release: Mutex::new(Some(gate_continue)),
            }),
        );
        let handle = manager
            .request(descriptor.clone(), MapDemand::Images)
            .unwrap();
        let started = Instant::now();
        loop {
            match batch_wait.recv_timeout(Duration::from_millis(10)) {
                Ok(()) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    let status = handle.status();
                    assert!(
                        !is_terminal(&status),
                        "map stopped before its second batch: {status:?}"
                    );
                    assert!(started.elapsed() < Duration::from_secs(15));
                }
                Err(error) => panic!("native batch gate disconnected: {error}"),
            }
        }
        let ticket = manager
            .inner
            .jobs
            .lock()
            .active
            .as_ref()
            .and_then(|active| active.ticket)
            .unwrap()
            .work_ticket();
        let shared = WorkService::shared().unwrap();
        let partial = root.partial_dir(
            descriptor.revision(),
            ArtifactKind::Images,
            descriptor.image_key().unwrap(),
        );
        let checkpoint_path = partial.join("checkpoint.json");
        let checkpoint_bytes = fs::read(&checkpoint_path).unwrap();
        let checkpoint = Checkpoint::decode(
            &checkpoint_bytes,
            ArtifactIdentity::Image(descriptor.image_identity()),
        )
        .unwrap();
        assert_eq!(
            checkpoint.completed.as_slice().len(),
            CHECKPOINT_BATCH_TILES
        );
        let uncheckpointed: Vec<_> = reference_images
            .manifest()
            .tiles
            .as_slice()
            .iter()
            .filter(|tile| {
                let key = UnitKey::Terrain { tile: tile.key };
                !checkpoint
                    .completed
                    .as_slice()
                    .iter()
                    .any(|unit| unit.key == key)
                    && partial.join(tile.key.relative_path().unwrap()).is_file()
            })
            .map(|tile| {
                (
                    tile.key,
                    file_id(&partial.join(tile.key.relative_path().unwrap())),
                )
            })
            .collect();
        assert!(!uncheckpointed.is_empty(), "the second batch did not begin");
        {
            // The native cursor gate holds this request in its active step.
            let mut service = shared.lock();
            assert_eq!(service.state(&ticket), Some(JobState::Executing));
            assert_eq!(
                service.cancel(ticket, Cancellation::Operator),
                api::work::CancelOutcome::Requested
            );
        }
        gate_release.send(()).unwrap();
        WorkService::wait_terminal(&shared, &ticket).unwrap();
        wait_for_status(&handle, MapJobStatus::Paused);
        let paused_checkpoint_bytes = fs::read(&checkpoint_path).unwrap();
        let paused_checkpoint = Checkpoint::decode(
            &paused_checkpoint_bytes,
            ArtifactIdentity::Image(descriptor.image_identity()),
        )
        .unwrap();
        assert!(paused_checkpoint.completed.as_slice().len() >= CHECKPOINT_BATCH_TILES);
        assert!(
            paused_checkpoint.completed.as_slice().len()
                < reference_images.manifest().tiles.as_slice().len()
        );
        drop(handle);
        drop(manager);
        assert_eq!(
            fs::read(&checkpoint_path).unwrap(),
            paused_checkpoint_bytes,
            "Drop must not rewrite the paused checkpoint"
        );
        let resumed_manager = MapDemandManager::new(
            root.clone(),
            Arc::new(crate::map_producer::NativeMapProducer::new()),
        );
        let resumed = resumed_manager
            .request(descriptor.clone(), MapDemand::Images)
            .unwrap();
        wait_ready(&resumed);
        let ready = resumed.ready().unwrap();
        let images = ready.images.as_ref().unwrap();
        assert_eq!(
            images.manifest().encode().unwrap(),
            reference_images.manifest().encode().unwrap()
        );
        for tile in reference_images.manifest().tiles.as_slice() {
            assert_eq!(
                images.read_tile_arc(tile.key).unwrap(),
                reference_images.read_tile_arc(tile.key).unwrap(),
                "resumed native tile {key:?} differs",
                key = tile.key
            );
        }
        let published = root.image_dir(descriptor.image_identity()).unwrap();
        for (key, id) in uncheckpointed {
            assert_eq!(file_id(&published.join(key.relative_path().unwrap())), id);
        }
    }
}
