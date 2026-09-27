//! Pure producer entry points. The host owns leases, locking, cancellation,
//! publication, and cache eviction; this module only borrows verified inputs.

use super::catalogue;
pub use super::catalogue::CatalogueStats;
use super::formats::ClientPois;
use super::identity::{CatalogueIdentity, CataloguePolicy, Digest};
use super::{MapError, Text};
use sha2::{Digest as _, Sha256};
use std::path::Path;

/// Borrowed view of one verified prepared client snapshot.
///
/// The host must retain the cloned `Arc<PreparedRuntimeCache>` that owns these
/// paths for the entire `'a` borrow and every producer call using it. A path is
/// never a lease: this type deliberately cannot outlive its caller's owner and
/// the map producer never stores or discovers a global/default cache path.
#[derive(Debug, Clone, Copy)]
pub struct ClientMapInput<'a> {
    pub revision: u16,
    pub content: Digest,
    pub jag_dir: &'a Path,
    pub snapshot_dir: &'a Path,
}

impl<'a> ClientMapInput<'a> {
    pub fn new(
        revision: u16,
        content: Digest,
        jag_dir: &'a Path,
        snapshot_dir: &'a Path,
    ) -> Result<Self, MapError> {
        super::revision(revision)?;
        if jag_dir.as_os_str().is_empty() || snapshot_dir.as_os_str().is_empty() {
            return Err(MapError::Path);
        }
        Ok(Self {
            revision,
            content,
            jag_dir,
            snapshot_dir,
        })
    }
}

pub fn catalogue_policy() -> CataloguePolicy {
    CataloguePolicy {
        algorithm: Text::new("client-loc-mapfunction-v1").expect("static policy text"),
        producer_sources: source_digest(&[
            include_bytes!("catalogue.rs"),
            include_bytes!("client_cache.rs"),
            include_bytes!("poi.rs"),
            include_bytes!("records.rs"),
            include_bytes!("../collision.rs"),
            include_bytes!("../../../../vendor/fr-client-rust/crates/client/src/map_cache.rs"),
            include_bytes!(
                "../../../../vendor/fr-client-rust/crates/client/src/config/loc_type.rs"
            ),
            include_bytes!(
                "../../../../vendor/fr-client-rust/crates/client/src/config/npc_type.rs"
            ),
        ]),
    }
}

pub fn catalogue_identity(input: ClientMapInput<'_>) -> CatalogueIdentity {
    CatalogueIdentity {
        revision: input.revision,
        content: input.content,
        policy: catalogue_policy().identity(),
    }
}

pub fn derive_catalogue(
    input: ClientMapInput<'_>,
) -> Result<(ClientPois, CatalogueStats), MapError> {
    catalogue::derive_client_pois(input, catalogue_identity(input))
}

pub(crate) fn source_digest(sources: &[&[u8]]) -> Digest {
    let mut hash = Sha256::new();
    hash.update(b"274bot.map.producer-sources\0");
    for source in sources {
        hash.update((source.len() as u64).to_be_bytes());
        hash.update(source);
    }
    Digest(hash.finalize().into())
}
