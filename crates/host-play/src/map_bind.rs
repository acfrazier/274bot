//! Process-wide WalkTo map demand. Panel and TUI call this; there is no per-bot state.
//!
//! The [`MapDemandManager`] is created on first map open, never at boot.

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
