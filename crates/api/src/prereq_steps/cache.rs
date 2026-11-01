//! Process-shared cache for the selected prerequisite-step family.
//!
//! The family is hash-verified off-pump during preparation. Cache-only lookup
//! never decodes or hashes the artifact.

use super::{wire, PrereqCatalog};
use crate::family_asset;
use crate::game_data::SelectedGameData;
use crate::selected::{
    ClientRevision, FactError, FamilyPreparation, SelectedPin, PREREQ_STEPS_FAMILY,
};
use parking_lot::Mutex;
use std::sync::Arc;

const PREREQ_STEPS_289: &[u8] = include_bytes!("../../data/game-data/289/prereq_steps.json");

fn embedded(revision: ClientRevision) -> &'static [u8] {
    match revision {
        ClientRevision::R289 => PREREQ_STEPS_289,
        ClientRevision::R274 => &[],
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Key {
    revision: i32,
    manifest: [u8; 32],
}

impl Key {
    fn of(pin: &SelectedPin) -> Self {
        Self {
            revision: pin.revision.as_i32(),
            manifest: pin.manifest_sha256,
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
            .ok_or(FactError::PinMismatch)?;
        let catalog = Arc::new(load()?);
        self.slots.lock()[empty] = Some(Slot {
            key,
            catalog: Arc::clone(&catalog),
        });
        Ok(catalog)
    }
}

static CACHE: Cache = Cache::new();

pub(crate) fn prepare(
    data: &SelectedGameData,
    _worker: &mut FamilyPreparation,
) -> Result<Arc<PrereqCatalog>, FactError> {
    let pin = data.selected_pin()?;
    CACHE.prepare(Key::of(&pin), || {
        load(&pin, crate::game_data::MANIFEST, embedded(pin.revision))
    })
}

pub(crate) fn cached(data: &SelectedGameData) -> Option<Arc<PrereqCatalog>> {
    let pin = data.selected_pin().ok()?;
    CACHE.lookup(&Key::of(&pin))
}

fn load(pin: &SelectedPin, manifest: &[u8], family: &[u8]) -> Result<PrereqCatalog, FactError> {
    family_asset::verify_family(pin, manifest, &PREREQ_STEPS_FAMILY, wire::SCHEMA, family)?;
    wire::decode(pin, family)
}
