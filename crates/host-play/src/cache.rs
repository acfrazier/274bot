//! Ordered runtime cache preparation for the selected server profile.
//!
//! The client crate owns the snapshot mechanics — version/CRC validation, the
//! staged publish with the manifest last, the single-flight preparation and
//! the cold fill through the update protocol. This module owns the host's
//! ordering on the normal application path (before the cache identity is
//! captured and the shared template is decoded), the progress stages that make
//! cold, warm and degraded starts distinguishable, and the availability facts
//! kept on the bound profile.

use std::path::Path;

use client::unpack::{self, FetchEndpoint, SnapshotPreparation, SnapshotState};
use client::BotTarget;

use crate::progress::{ProfileProgress, ProfileProgressObserver, ProfileProgressStage};

/// Prepared-snapshot facts for the selected cache version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheAvailability {
    /// A complete snapshot for the selected version exists (warm reuse, or a
    /// snapshot filled earlier in this process) or was published by this
    /// preparation (cold start or repair).
    Ready {
        version: String,
        /// True when this preparation published the snapshot.
        published: bool,
        /// `local-store` (the engine's `main_file_cache`) or `update-server`
        /// (the cold fill through the OnDemand worker).
        source: &'static str,
    },
    /// No usable snapshot after every available source was tried: clients keep
    /// the OnDemand fallback. The reason names the failing check.
    Degraded(String),
}

/// What one preparation did for the selected profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachePreparation {
    pub availability: CacheAvailability,
    /// Jag packs fetched from the update server by this preparation.
    pub fetched: Vec<String>,
}

impl CachePreparation {
    pub fn is_ready(&self) -> bool {
        matches!(self.availability, CacheAvailability::Ready { .. })
    }
}

/// Check the selected version's snapshot and, when it is not already complete,
/// fetch genuinely missing pack files (the real CRC-checked update-server
/// fetch) and prepare it: unpack from the local store, or report the degraded
/// fallback when only the client's own OnDemand worker could fill it.
///
/// Repeated calls are cheap: a warm snapshot is returned by the state check,
/// and the client crate records a completed preparation for the process.
pub fn prepare(
    cache_dir: &Path,
    unpack_dir: &Path,
    target: BotTarget,
    asset_host: &str,
    asset_port: u16,
    observer: &ProfileProgressObserver,
) -> CachePreparation {
    let cache = cache_dir.to_string_lossy().into_owned();
    let unpack_root = unpack_dir.to_string_lossy().into_owned();

    observer.report(ProfileProgress::steps(
        ProfileProgressStage::CheckingCache,
        0,
        1,
    ));
    let state = unpack::snapshot_state(&cache, &unpack_root);
    observer.report(ProfileProgress::steps(
        ProfileProgressStage::CheckingCache,
        1,
        1,
    ));

    if let Ok(SnapshotState::Ready(manifest)) = state {
        return CachePreparation {
            availability: CacheAvailability::Ready {
                version: manifest.version,
                published: false,
                source: if manifest.source == "update-server" {
                    "update-server"
                } else {
                    "local-store"
                },
            },
            fetched: Vec::new(),
        };
    }

    observer.report(ProfileProgress::steps(
        ProfileProgressStage::PreparingCache,
        0,
        1,
    ));
    let prepared = unpack::prepare_snapshot(
        &cache,
        &unpack_root,
        Some(FetchEndpoint {
            target,
            host: asset_host,
            port: asset_port,
        }),
        // The bind path has no client worker yet: a cache that only the update
        // protocol can fill is reported here and completed at client boot,
        // where the session's OnDemand worker supplies the entries.
        None,
    );
    observer.report(ProfileProgress::steps(
        ProfileProgressStage::PreparingCache,
        1,
        1,
    ));

    match &*prepared {
        SnapshotPreparation::Ready {
            version,
            published,
            source,
            fetched,
            ..
        } => CachePreparation {
            availability: CacheAvailability::Ready {
                version: version.clone(),
                published: *published,
                source,
            },
            fetched: fetched.clone(),
        },
        SnapshotPreparation::Unavailable { reason } => CachePreparation {
            availability: CacheAvailability::Degraded(reason.clone()),
            fetched: Vec::new(),
        },
    }
}
