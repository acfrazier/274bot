//! Manifest assembly for release navigation artifacts.

use nav_planner::manifest::{hash_bytes, CacheManifest, NavManifest};

use crate::GenerationError;

/// Bind generated pack and sidecar bytes to their selected cache identity.
#[expect(
    clippy::too_many_arguments,
    reason = "the manifest binds every existing artifact and zone count"
)]
pub fn capture_nav_manifest(
    revision: u16,
    cache: &CacheManifest,
    nav: &[u8],
    flags: Option<&[u8]>,
    reach: Option<&[u8]>,
    canlight: Option<&[u8]>,
    pois: Option<&[u8]>,
    zone_count: u32,
    zone_npc_count: u32,
) -> Result<NavManifest, GenerationError> {
    if revision != 274 && revision != 289 {
        return Err(GenerationError::UnsupportedRevision { revision });
    }
    if cache.revision != revision {
        return Err(GenerationError::CacheManifestRevisionMismatch {
            manifest_revision: cache.revision,
            selected_revision: revision,
        });
    }
    Ok(NavManifest {
        revision,
        cache_id: cache.identity(),
        nav_sha256: hash_bytes(nav),
        zone_count,
        zone_npc_count,
        flags_sha256: flags.map(hash_bytes),
        reach_sha256: reach.map(hash_bytes),
        canlight_sha256: canlight.map(hash_bytes),
        pois_sha256: pois.map(hash_bytes),
        content_id: None,
        source_sha256: None,
    })
}

#[cfg(test)]
#[path = "manifest_tests.rs"]
mod tests;
