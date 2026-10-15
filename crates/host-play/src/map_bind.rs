//! Process-wide WalkTo map demand. Panel and TUI call this; there is no per-bot state.
//!
//! The [`MapDemandManager`] is created on first map open, never at boot.

use api::work::{
    Checkpoint, CooperativeJob, RetainedState, StepContext, StepOutcome, WalkMapDecodePhase,
};
use crate::map_cache::{
    MapCacheError, MapCacheRoot, MapDemand, MapDemandHandle, MapDemandManager, MapJobStatus,
    MapProfileDescriptor, PreparedMapInput, ReadyCatalogue, ReadyImages, ReadyMap,
    ShippedMapImages,
};
use crate::map_producer::{map_artifact_policies, NativeMapProducer};
use crate::nav_identity::install_resource_root;
use crate::walk_map::AuthenticatedServices;
use crate::ServerProfile;
use nav::map::formats::{ServicePois, ShippedImages, NAVPOIS_HEADER_BYTES};
use nav::map::identity::Digest;
use std::fs;
use std::io::Cursor;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

pub use crate::map_producer::map_artifact_policies as artifact_policies;

static MANAGER: OnceLock<MapDemandManager> = OnceLock::new();

/// Directory of the release's shipped map terrain under an install resource
/// root (next to `nav/`).
pub const SHIPPED_MAP_DIR: &str = "map";

/// One process-wide manager. First explicit map open creates it against
/// `~/.274bot/map-cache` and the native producer; terrain shipped under the
/// install resource root's `map/` is installed when it matches exactly.
pub fn map_demand_manager() -> Result<&'static MapDemandManager, MapCacheError> {
    if let Some(manager) = MANAGER.get() {
        return Ok(manager);
    }
    let root = MapCacheRoot::current()?;
    let producer = Arc::new(NativeMapProducer::new());
    let created = match std::env::current_exe() {
        Ok(exe) => MapDemandManager::with_shipped_images(
            root,
            producer,
            ShippedMapImages::at(install_resource_root(&exe).join(SHIPPED_MAP_DIR)),
        ),
        Err(_) => MapDemandManager::new(root, producer),
    };
    Ok(MANAGER.get_or_init(|| created))
}

/// Join a finished bake thread on close. Never creates the manager: a map
/// that was never opened leaves nothing to reap.
pub fn reap_map_demand() {
    if let Some(manager) = MANAGER.get() {
        manager.reap();
    }
}

/// Descriptor for the bound profile: image/catalogue policies from B plus a
/// second `Arc` on the prepared client cache.
pub fn map_profile_descriptor(
    profile: &ServerProfile,
) -> Result<MapProfileDescriptor, MapCacheError> {
    let (image, catalogue) = map_artifact_policies()?;
    MapProfileDescriptor::from_profile(profile, image, catalogue)
}

/// Request `demand` for `descriptor`, starting or joining the local worker
/// when it is not ready. Images demand always runs catalogue first (D's
/// worker).
///
/// Reopening a failed/paused identity is an explicit retry, not an automatic
/// loop: ordinary `request` keeps the terminal status.
///
/// Front ends open through `frontend_core::MapBakeGate`, which asks the
/// operator before this starts a local terrain bake.
pub fn open_map_demand(
    manager: &MapDemandManager,
    descriptor: MapProfileDescriptor,
    demand: MapDemand,
) -> Result<MapDemandHandle, MapCacheError> {
    let handle = manager.request(descriptor.clone(), demand)?;
    match handle.status() {
        MapJobStatus::Failed(_) | MapJobStatus::Paused => manager.retry(descriptor, demand),
        _ => Ok(handle),
    }
}

/// Open terrain that needs no local bake for `descriptor` without ever
/// baking it: terrain already published, or shipped with this release for
/// exactly its image identity (the worker installs that). `None` when there
/// is neither. A published terrain stays leased for the handle, so capacity
/// pruning cannot remove it while the worker derives a missing catalogue,
/// and the [`MapDemand::ReadyImages`] image stage never starts a bake: it
/// settles catalogue-only if the terrain is gone or the shipped files are
/// unusable, and the front end asks before baking.
pub fn open_map_ready_terrain(
    manager: &MapDemandManager,
    descriptor: MapProfileDescriptor,
) -> Result<Option<MapDemandHandle>, MapCacheError> {
    if let Some(terrain) = manager.ready_images(&descriptor)? {
        let handle = open_map_demand(manager, descriptor, MapDemand::ReadyImages)?;
        return Ok(Some(handle.with_terrain_pin(terrain)));
    }
    if manager.shipped_images_available(&descriptor) {
        return open_map_demand(manager, descriptor, MapDemand::ReadyImages).map(Some);
    }
    Ok(None)
}

/// Release packaging: bake `revision` terrain for an offline client cache
/// (`jag_dir` and the decoded snapshot under `snapshot_root`, as the nav
/// build reads them) through the production image path, then ship it under
/// `out/map/<revision>/` for the install resource root `out`. `progress`
/// sees every worker status until the bake settles.
pub fn bake_shipped_map_images(
    revision: u16,
    jag_dir: &Path,
    snapshot_root: &Path,
    out: &Path,
    progress: impl FnMut(&MapJobStatus),
) -> Result<ShippedImages, MapCacheError> {
    let input = PreparedMapInput::offline(revision, jag_dir, snapshot_root)?;
    ship_map_images(input, out, progress)
}

pub(crate) fn ship_map_images(
    input: PreparedMapInput,
    out: &Path,
    mut progress: impl FnMut(&MapJobStatus),
) -> Result<ShippedImages, MapCacheError> {
    let (image, catalogue) = map_artifact_policies()?;
    let descriptor =
        MapProfileDescriptor::new(input.revision(), input.content(), image, catalogue)?
            .with_input(input)?;
    let shipped = ShippedMapImages::at(out.join(SHIPPED_MAP_DIR));
    if shipped.description_path(descriptor.revision()).exists() {
        return Err(MapCacheError::Message(format!(
            "shipped map terrain already exists under {}",
            shipped.path().display()
        )));
    }
    // A scratch cache on the output's volume: the same manager, producer,
    // writer and publication as a local bake.
    let scratch = out.join(format!(".map-bake-{}", std::process::id()));
    let root = MapCacheRoot::from_root(&scratch);
    let result = (|| {
        let manager = MapDemandManager::new(root.clone(), Arc::new(NativeMapProducer::new()));
        let handle = manager.request(descriptor.clone(), MapDemand::Images)?;
        loop {
            let status = handle.status();
            progress(&status);
            match status {
                MapJobStatus::Ready => break,
                MapJobStatus::Failed(message) => return Err(MapCacheError::Message(message)),
                MapJobStatus::Paused | MapJobStatus::Cancelled => {
                    return Err(MapCacheError::Cancelled)
                }
                MapJobStatus::Queued | MapJobStatus::Running(_) => {}
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        drop(handle);
        // Join the worker so none of its leases outlives the scratch cache.
        while !manager.reap_idle() {
            std::thread::sleep(Duration::from_millis(10));
        }
        shipped.ship_from(&root, descriptor.image_identity())
    })();
    match (result, fs::remove_dir_all(&scratch)) {
        (Ok(_), Err(error)) if error.kind() != std::io::ErrorKind::NotFound => Err(
            MapCacheError::Message(format!("scratch map cache {}: {error}", scratch.display())),
        ),
        (result, _) => result,
    }
}

/// Catalogue published independently of terrain. `None` while still baking.
pub fn peek_map_catalogue(profile: &ServerProfile) -> Option<Arc<ReadyCatalogue>> {
    let descriptor = map_profile_descriptor(profile).ok()?;
    map_demand_manager()
        .ok()?
        .open_ready(&descriptor, MapDemand::CatalogueOnly)
        .ok()
        .map(|ready| ready.catalogue)
}

pub fn poll_map_status(handle: &MapDemandHandle) -> MapJobStatus {
    handle.status()
}

pub fn map_ready(handle: &MapDemandHandle) -> Result<ReadyMap, MapCacheError> {
    handle.ready()
}

pub fn map_ready_catalogue(handle: &MapDemandHandle) -> Result<Arc<ReadyCatalogue>, MapCacheError> {
    Ok(handle.ready()?.catalogue)
}

pub fn map_ready_images(
    handle: &MapDemandHandle,
) -> Result<Option<Arc<ReadyImages>>, MapCacheError> {
    Ok(handle.ready()?.images)
}

/// Maximum compressed terrain bytes admitted to the shared walk-map decoder.
pub const WALK_MAP_DECODE_PNG_LIMIT: usize = 512 * 1024;
/// WorkAdmission scratch reservation for png 0.18.1: 512 KiB parser limits
/// for raw chunks, one bounded eXIf copy, ≤136 KiB row buffering, and inflater state.
pub const WALK_MAP_DECODE_SCRATCH_LIMIT: usize = 2 * 1024 * 1024;
const PNG_DECODER_LIMIT_BYTES: usize = 512 * 1024;

/// Stable, allocation-free failures from the bounded walk-map PNG decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkMapDecodeFailure {
    Truncated,
    PngHeader,
    DimensionsOrFormat,
    PngBytesLimit,
    PngInfo,
    PngOutputLimit,
    PngOutputSize,
    PngFrame,
    PngFrameSize,
    PngTrailing,
    AlreadyRun,
}

impl std::fmt::Display for WalkMapDecodeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Truncated => "truncated map data",
            Self::PngHeader => "invalid map PNG header",
            Self::DimensionsOrFormat => "invalid map PNG dimensions/format",
            Self::PngBytesLimit => "map limit exceeded: png bytes",
            Self::PngInfo => "invalid map png decode info",
            Self::PngOutputLimit => "map limit exceeded: png output",
            Self::PngOutputSize => "invalid map png output size",
            Self::PngFrame => "invalid map png frame",
            Self::PngFrameSize => "invalid map png frame size",
            Self::PngTrailing => "invalid map PNG trailing chunks",
            Self::AlreadyRun => "walk map decode job was already run",
        })
    }
}

impl std::error::Error for WalkMapDecodeFailure {}

/// One bounded PNG decode on the process-wide WorkAdmission worker.
pub struct WalkMapDecodeJob {
    png: Option<Arc<[u8]>>,
    decoded: Option<Result<Vec<u8>, WalkMapDecodeFailure>>,
}

impl WalkMapDecodeJob {
    /// Transfer immutable cache bytes into the cooperative decode job.
    pub fn new(png: Arc<[u8]>) -> Self {
        Self {
            png: Some(png),
            decoded: None,
        }
    }

    fn publish_decoded(
        &mut self,
        ctx: &mut StepContext<'_, WalkMapDecodePhase>,
    ) -> StepOutcome<Vec<u8>, WalkMapDecodeFailure> {
        let Some(decoded) = self.decoded.take() else {
            return StepOutcome::Failed(WalkMapDecodeFailure::AlreadyRun);
        };
        ctx.publish_progress(WalkMapDecodePhase::Publish, 1, Some(1));
        match ctx.checkpoint() {
            Checkpoint::Continue => match decoded {
                Ok(rgba) => StepOutcome::Completed(rgba),
                Err(error) => StepOutcome::Failed(error),
            },
            Checkpoint::Yield | Checkpoint::Cancel(_) => {
                self.decoded = Some(decoded);
                StepOutcome::Yielded
            }
        }
    }
}

impl CooperativeJob for WalkMapDecodeJob {
    type Output = Vec<u8>;
    type Error = WalkMapDecodeFailure;
    type Phase = WalkMapDecodePhase;

    fn step(
        &mut self,
        ctx: &mut StepContext<'_, WalkMapDecodePhase>,
    ) -> StepOutcome<Self::Output, Self::Error> {
        match ctx.checkpoint() {
            Checkpoint::Continue => {}
            Checkpoint::Yield | Checkpoint::Cancel(_) => return StepOutcome::Yielded,
        }
        if self.decoded.is_some() {
            return self.publish_decoded(ctx);
        }
        ctx.publish_progress(WalkMapDecodePhase::Receive, 0, Some(1));
        match ctx.consume_unit() {
            Checkpoint::Continue => {}
            Checkpoint::Yield | Checkpoint::Cancel(_) => return StepOutcome::Yielded,
        }
        let Some(png) = self.png.take() else {
            return StepOutcome::Failed(WalkMapDecodeFailure::AlreadyRun);
        };
        ctx.publish_progress(WalkMapDecodePhase::Decode, 0, Some(1));
        self.decoded = Some(decode_walk_map_png(&png));
        drop(png);
        match ctx.checkpoint() {
            Checkpoint::Continue => self.publish_decoded(ctx),
            Checkpoint::Yield | Checkpoint::Cancel(_) => StepOutcome::Yielded,
        }
    }

    fn retained(&self) -> RetainedState {
        let input_bytes = self.png.as_ref().map_or(0, |png| png.len());
        let result_bytes = self
            .decoded
            .as_ref()
            .and_then(|decoded| decoded.as_ref().ok())
            .map_or(0, Vec::len);
        RetainedState::clean(input_bytes + result_bytes + std::mem::size_of::<Self>())
    }
}

fn decode_walk_map_png(bytes: &[u8]) -> Result<Vec<u8>, WalkMapDecodeFailure> {
    use nav::map::spatial::{TILE_PIXELS, TILE_RGBA_BYTES};

    if bytes.len() < 33 {
        return Err(WalkMapDecodeFailure::Truncated);
    }
    if &bytes[..8] != b"\x89PNG\r\n\x1a\n"
        || bytes[8..12] != 13u32.to_be_bytes()
        || &bytes[12..16] != b"IHDR"
    {
        return Err(WalkMapDecodeFailure::PngHeader);
    }
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    if width != TILE_PIXELS || height != TILE_PIXELS || bytes[24..29] != [8, 6, 0, 0, 0] {
        return Err(WalkMapDecodeFailure::DimensionsOrFormat);
    }
    if bytes.len() > WALK_MAP_DECODE_PNG_LIMIT {
        return Err(WalkMapDecodeFailure::PngBytesLimit);
    }
    // The former decoder applied no ICC/gamma transformation, so unused text
    // and ICC metadata cannot affect RGBA. Skip text/ICC parsing and inflation;
    // png retains one bounded eXIf copy, included in the WorkAdmission charge.
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_ignore_text_chunk(true);
    decoder.set_ignore_iccp_chunk(true);
    decoder.set_limits(png::Limits {
        bytes: PNG_DECODER_LIMIT_BYTES,
    });
    let mut reader = decoder
        .read_info()
        .map_err(|_| WalkMapDecodeFailure::PngInfo)?;
    let size = reader
        .output_buffer_size()
        .ok_or(WalkMapDecodeFailure::PngOutputLimit)?;
    if size != TILE_RGBA_BYTES {
        return Err(WalkMapDecodeFailure::PngOutputSize);
    }
    let mut rgba = vec![0u8; TILE_RGBA_BYTES];
    let info = reader
        .next_frame(&mut rgba)
        .map_err(|_| WalkMapDecodeFailure::PngFrame)?;
    if info.width != TILE_PIXELS
        || info.height != TILE_PIXELS
        || info.buffer_size() != TILE_RGBA_BYTES
    {
        return Err(WalkMapDecodeFailure::PngFrameSize);
    }
    reader
        .finish()
        .map_err(|_| WalkMapDecodeFailure::PngTrailing)?;
    Ok(rgba)
}

/// Authenticated navpois next to the bound pack. Missing/stale/wrong-identity
/// is `None` — callers keep that explicit.
pub fn load_navpois(
    profile: &ServerProfile,
    content: Digest,
    nav: Digest,
) -> Option<AuthenticatedServices> {
    let identity = profile.nav_identity()?;
    let expected_file = Digest::from_hex(identity.pois_sha256.as_deref()?).ok()?;
    let pack = profile.nav_pack();
    let candidates = [
        pack.with_extension("navpois"),
        pack.with_file_name("274bot.navpois"),
    ];
    let bytes = candidates.iter().find_map(|path| fs::read(path).ok())?;
    if Digest::of(&bytes) != expected_file || bytes.len() < NAVPOIS_HEADER_BYTES {
        return None;
    }
    let document: ServicePois = serde_json::from_slice(&bytes[NAVPOIS_HEADER_BYTES..]).ok()?;
    let services = AuthenticatedServices::decode(&bytes, document.identity, expected_file).ok()?;
    let bound = services.document().identity;
    if bound.revision != profile.revision().as_i32() as u16
        || bound.content != content
        || bound.nav_sha256 != nav
    {
        return None;
    }
    Some(services)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map_cache::{MapCacheRoot, MapJobStatus, MapProfileDescriptor};
    use nav::map::identity::Digest;
    use std::time::{Duration, Instant};

    #[test]
    fn artifact_policies_are_stable() {
        let a = map_artifact_policies().unwrap();
        let b = map_artifact_policies().unwrap();
        assert_eq!(a, b);
        assert_ne!(a.0, a.1);
    }

    #[test]
    fn native_producer_fails_closed_without_prepared_cache() {
        let root = MapCacheRoot::from_root(
            std::env::temp_dir().join(format!("274bot-native-producer-{}", std::process::id())),
        );
        let manager = MapDemandManager::new(root, Arc::new(NativeMapProducer::new()));
        let (image, catalogue) = map_artifact_policies().unwrap();
        let descriptor = MapProfileDescriptor::new(289, Digest([1; 32]), image, catalogue).unwrap();
        let handle = manager.request_catalogue(descriptor).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let status = loop {
            let status = handle.status();
            if !matches!(status, MapJobStatus::Queued | MapJobStatus::Running(_)) {
                break status;
            }
            if Instant::now() > deadline {
                panic!("producer did not finish: {status:?}");
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        match status {
            MapJobStatus::Failed(message) => {
                assert!(
                    message.contains("prepared client cache")
                        || message.contains("map assets unavailable"),
                    "{message}"
                );
            }
            other => panic!("expected failed without cache, got {other:?}"),
        }
    }
}
