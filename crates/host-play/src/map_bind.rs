//! Process-wide WalkTo map demand. Panel and TUI call this; there is no per-bot state.
//!
//! The [`MapDemandManager`] is created on first map open, never at boot.

use crate::map_cache::{
    MapCacheError, MapCacheRoot, MapDemand, MapDemandHandle, MapDemandManager, MapJobStatus,
    MapProfileDescriptor, ReadyCatalogue, ReadyImages, ReadyMap,
};
use crate::map_producer::{map_artifact_policies, NativeMapProducer};
use crate::walk_map::AuthenticatedServices;
use crate::ServerProfile;
use nav::map::formats::{ServicePois, NAVPOIS_HEADER_BYTES};
use nav::map::identity::Digest;
use std::fs;
use std::sync::{Arc, OnceLock};

pub use crate::map_producer::map_artifact_policies as artifact_policies;

static MANAGER: OnceLock<MapDemandManager> = OnceLock::new();

/// One process-wide manager. First explicit map open creates it against
/// `~/.274bot/map-cache` and the native producer.
pub fn map_demand_manager() -> Result<&'static MapDemandManager, MapCacheError> {
    if let Some(manager) = MANAGER.get() {
        return Ok(manager);
    }
    let root = MapCacheRoot::current()?;
    let created = MapDemandManager::new(root, Arc::new(NativeMapProducer::new()));
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

/// Open terrain that is already published for `descriptor` without ever
/// baking it: `None` when there is none. The handle keeps the terrain leased
/// while the worker derives a missing catalogue, so capacity pruning cannot
/// remove it, and its [`MapDemand::ReadyImages`] image stage never starts a
/// bake (it settles catalogue-only if the terrain is gone anyway).
pub fn open_map_ready_terrain(
    manager: &MapDemandManager,
    descriptor: MapProfileDescriptor,
) -> Result<Option<MapDemandHandle>, MapCacheError> {
    let Some(terrain) = manager.ready_images(&descriptor)? else {
        return Ok(None);
    };
    let handle = open_map_demand(manager, descriptor, MapDemand::ReadyImages)?;
    Ok(Some(handle.with_terrain_pin(terrain)))
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
