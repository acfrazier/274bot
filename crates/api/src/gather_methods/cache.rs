//! Process-shared bounded cache of prepared gathering catalogs, keyed by the selected revision.
//!
//! Preparation runs off-pump through the `FamilyPreparation` capability. The
//! first caller decodes the local family; later callers share the immutable
//! catalog. Cache-only lookup never decodes or waits for preparation.

use super::catalog::GatherCatalog;
use super::wire;
use crate::game_data::SelectedGameData;
use crate::selected::{ClientRevision, FactError, FamilyPreparation, GATHERING_FAMILY};
use parking_lot::Mutex;
use std::sync::Arc;

#[cfg(all(feature = "embedded-game-data", not(no_local_game_data)))]
const GATHERING_274: &[u8] = include_bytes!("../../data/game-data/274/gathering.json");
#[cfg(all(feature = "embedded-game-data", not(no_local_game_data)))]
const GATHERING_289: &[u8] = include_bytes!("../../data/game-data/289/gathering.json");
#[cfg(all(feature = "embedded-game-data", not(no_local_game_data)))]
const GATHERING_377: &[u8] = include_bytes!("../../data/game-data/377/gathering.json");

#[cfg(all(feature = "embedded-game-data", not(no_local_game_data)))]
fn embedded(revision: ClientRevision) -> Option<&'static [u8]> {
    Some(match revision {
        ClientRevision::R274 => GATHERING_274,
        ClientRevision::R289 => GATHERING_289,
        ClientRevision::R377 => GATHERING_377,
    })
}

#[cfg(not(all(feature = "embedded-game-data", not(no_local_game_data))))]
fn embedded(_revision: ClientRevision) -> Option<&'static [u8]> {
    None
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Key {
    revision: i32,
}

impl Key {
    fn of(revision: ClientRevision) -> Self {
        Self {
            revision: revision.as_i32(),
        }
    }
}

struct Slot {
    key: Key,
    catalog: Arc<GatherCatalog>,
}

/// Slots are only touched under brief locks; `preparing` serializes decodes
/// while `lookup` (the tick path) never waits behind a decode.
struct Cache {
    slots: Mutex<[Option<Slot>; 3]>,
    preparing: Mutex<()>,
}

impl Cache {
    const fn new() -> Self {
        Self {
            slots: Mutex::new([None, None, None]),
            preparing: Mutex::new(()),
        }
    }

    fn lookup(&self, key: &Key) -> Option<Arc<GatherCatalog>> {
        self.slots
            .lock()
            .iter()
            .flatten()
            .find(|slot| slot.key == *key)
            .map(|slot| Arc::clone(&slot.catalog))
    }

    fn prepare(
        &self,
        key: Key,
        load: impl FnOnce() -> Result<GatherCatalog, FactError>,
    ) -> Result<Arc<GatherCatalog>, FactError> {
        if let Some(hit) = self.lookup(&key) {
            return Ok(hit);
        }
        let _single_flight = self.preparing.lock();
        if let Some(hit) = self.lookup(&key) {
            return Ok(hit);
        }
        let empty = self
            .slots
            .lock()
            .iter()
            .position(Option::is_none)
            .ok_or_else(unavailable)?;
        let catalog = Arc::new(load()?);
        self.slots.lock()[empty] = Some(Slot {
            key,
            catalog: Arc::clone(&catalog),
        });
        Ok(catalog)
    }
}

static CACHE: Cache = Cache::new();

fn unavailable() -> FactError {
    FactError::FamilyUnavailable((*GATHERING_FAMILY).clone())
}

/// Decode the selected revision's gathering family once per process, or share
/// the immutable catalog another user has prepared.
pub fn prepare(
    data: &SelectedGameData,
    _worker: &mut FamilyPreparation,
) -> Result<Arc<GatherCatalog>, FactError> {
    let revision = data.selected_revision().ok_or_else(unavailable)?;
    CACHE.prepare(Key::of(revision), || {
        let family = embedded(revision).ok_or_else(unavailable)?;
        load(family)
    })
}

/// Return the prepared catalog if it is cached. Never decodes or waits.
pub fn cached(data: &SelectedGameData) -> Option<Arc<GatherCatalog>> {
    let revision = data.selected_revision()?;
    CACHE.lookup(&Key::of(revision))
}

fn load(family: &[u8]) -> Result<GatherCatalog, FactError> {
    wire::decode(family)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::borrow::Cow;
    use std::time::Instant;

    fn real(revision: ClientRevision) -> Option<(Cow<'static, [u8]>, usize)> {
        let (path, description) = match revision {
            ClientRevision::R274 => (
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/data/game-data/274/gathering.json"
                ),
                "revision 274 gathering family for API tests",
            ),
            ClientRevision::R289 => (
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/data/game-data/289/gathering.json"
                ),
                "revision 289 gathering family for API tests",
            ),
            ClientRevision::R377 => (
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/data/game-data/377/gathering.json"
                ),
                "revision 377 gathering family for API tests",
            ),
        };
        let path = crate::test_support::local_asset_path("", path, description)?;
        if let Some(bytes) = embedded(revision) {
            return Some((Cow::Borrowed(bytes), bytes.len()));
        }
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        let len = bytes.len();
        Some((Cow::Owned(bytes), len))
    }

    #[test]
    fn a_prepared_catalog_is_shared_after_the_last_caller_drops() {
        let cache = Cache::new();
        let key = Key::of(ClientRevision::R289);
        assert!(cache.lookup(&key).is_none(), "nothing decoded yet");
        let Some((bytes, _)) = real(ClientRevision::R289) else {
            return;
        };
        let first = cache.prepare(key, || load(&bytes)).unwrap();
        let second = cache
            .prepare(key, || panic!("a cached catalog must not be decoded again"))
            .unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        drop(first);
        drop(second);
        let again = cache
            .prepare(key, || panic!("the last caller cannot evict process cache"))
            .unwrap();
        assert!(Arc::ptr_eq(&again, &cache.lookup(&key).unwrap()));
    }

    #[test]
    fn cache_is_keyed_by_selected_revision_and_bounded_to_supported_revisions() {
        let cache = Cache::new();
        let key_274 = Key::of(ClientRevision::R274);
        let key_289 = Key::of(ClientRevision::R289);
        let key_377 = Key::of(ClientRevision::R377);
        let cold_274 = Instant::now();
        let Some((bytes_274, len_274)) = real(ClientRevision::R274) else {
            return;
        };
        let Some((bytes_289, len_289)) = real(ClientRevision::R289) else {
            return;
        };
        let catalog_274 = cache.prepare(key_274, || load(&bytes_274)).unwrap();
        println!(
            "gathering decode 274: input={}B cold={:?}",
            len_274,
            cold_274.elapsed()
        );
        let cold_289 = Instant::now();
        let catalog_289 = cache.prepare(key_289, || load(&bytes_289)).unwrap();
        println!(
            "gathering decode 289: input={}B cold={:?}",
            len_289,
            cold_289.elapsed()
        );
        let Some((bytes_377, len_377)) = real(ClientRevision::R377) else {
            return;
        };
        let cold_377 = Instant::now();
        let catalog_377 = cache.prepare(key_377, || load(&bytes_377)).unwrap();
        println!(
            "gathering decode 377: input={}B cold={:?}",
            len_377,
            cold_377.elapsed()
        );
        assert!(!Arc::ptr_eq(&catalog_274, &catalog_289));
        assert!(!Arc::ptr_eq(&catalog_274, &catalog_377));
        assert!(!Arc::ptr_eq(&catalog_289, &catalog_377));
        drop((catalog_274, catalog_289, catalog_377));
        assert!(cache.lookup(&key_274).is_some());
        assert!(cache.lookup(&key_289).is_some());
        assert!(cache.lookup(&key_377).is_some());
    }

    #[test]
    fn a_failed_preparation_is_not_cached() {
        let cache = Cache::new();
        let key = Key::of(ClientRevision::R289);
        let error = FactError::Schema {
            expected: wire::SCHEMA,
            actual: wire::SCHEMA + 1,
        };
        assert_eq!(
            cache.prepare(key, || Err(error.clone())).unwrap_err(),
            error
        );
        assert!(cache.lookup(&key).is_none());
        let Some((bytes, _)) = real(ClientRevision::R289) else {
            return;
        };
        assert!(cache.prepare(key, || load(&bytes)).is_ok());
    }
}
