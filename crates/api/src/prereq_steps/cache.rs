//! Process-shared cache for the selected prerequisite-step family.
//!
//! The family is hash-verified off-pump during preparation. Cache-only lookup
//! never decodes or hashes the artifact.

use super::{wire, PrereqCatalog};
use crate::game_data::SelectedGameData;
use crate::selected::{ClientRevision, FactError, FamilyPreparation, PREREQ_STEPS_FAMILY};
use parking_lot::Mutex;
use std::sync::Arc;

#[cfg(all(feature = "embedded-game-data", not(no_local_game_data)))]
const PREREQ_STEPS_289: &[u8] = include_bytes!("../../data/game-data/289/prereq_steps.json");

#[cfg(all(feature = "embedded-game-data", not(no_local_game_data)))]
fn embedded(revision: ClientRevision) -> Option<&'static [u8]> {
    match revision {
        ClientRevision::R289 => Some(PREREQ_STEPS_289),
        ClientRevision::R274 | ClientRevision::R377 => None,
    }
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
    catalog: Arc<PrereqCatalog>,
}

struct Cache {
    slots: Mutex<[Option<Slot>; 2]>,
    preparing: Mutex<()>,
}

impl Cache {
    const fn new() -> Self {
        Self {
            slots: Mutex::new([None, None]),
            preparing: Mutex::new(()),
        }
    }

    fn lookup(&self, key: &Key) -> Option<Arc<PrereqCatalog>> {
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
        load: impl FnOnce() -> Result<PrereqCatalog, FactError>,
    ) -> Result<Arc<PrereqCatalog>, FactError> {
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
    FactError::FamilyUnavailable((*PREREQ_STEPS_FAMILY).clone())
}

pub(crate) fn prepare(
    data: &SelectedGameData,
    _worker: &mut FamilyPreparation,
) -> Result<Arc<PrereqCatalog>, FactError> {
    let revision = data.selected_revision().ok_or_else(unavailable)?;
    CACHE.prepare(Key::of(revision), || {
        let family = embedded(revision).ok_or_else(unavailable)?;
        load(family)
    })
}

pub(crate) fn cached(data: &SelectedGameData) -> Option<Arc<PrereqCatalog>> {
    let revision = data.selected_revision()?;
    CACHE.lookup(&Key::of(revision))
}

fn load(family: &[u8]) -> Result<PrereqCatalog, FactError> {
    wire::decode(family)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_revisions_do_not_receive_289_prerequisites() {
        for revision in [ClientRevision::R274, ClientRevision::R377] {
            assert!(embedded(revision).is_none());
        }
    }
}
