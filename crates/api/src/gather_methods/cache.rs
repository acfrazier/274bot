//! Process-shared weak cache of the prepared gathering catalog, keyed by the selected manifest.
//!
//! Preparation (`prepare`) needs the off-pump `FamilyPreparation` capability: it verifies the embedded family file
//! against the pinned manifest and decodes it once. Every user then holds an `Arc`; when the last one drops, the
//! catalog is freed and only a dead `Weak` remains until the next preparation prunes it. The tick/UI path
//! (`cached`) never decodes, hashes or waits for a preparation in flight.

use super::catalog::GatherCatalog;
use super::wire;
use crate::family_asset;
use crate::game_data::{SelectedGameData, MANIFEST};
use crate::selected::{
    ClientRevision, FactError, FamilyPreparation, SelectedPin, GATHERING_FAMILY,
};
use parking_lot::Mutex;
use std::sync::{Arc, Weak};

const GATHERING_274: &[u8] = include_bytes!("../../data/game-data/274/gathering.json");
const GATHERING_289: &[u8] = include_bytes!("../../data/game-data/289/gathering.json");

fn embedded(revision: ClientRevision) -> &'static [u8] {
    match revision {
        ClientRevision::R274 => GATHERING_274,
        ClientRevision::R289 => GATHERING_289,
    }
}

/// One selected manifest of one revision: the identity a prepared catalog is valid for.
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
    catalog: Weak<GatherCatalog>,
}

/// Slots are only touched under brief locks; `preparing` serializes decodes so one revision is decoded once while
/// `lookup` (the tick path) never waits behind it.
struct Cache {
    slots: Mutex<Vec<Slot>>,
    preparing: Mutex<()>,
}

impl Cache {
    const fn new() -> Self {
        Self {
            slots: Mutex::new(Vec::new()),
            preparing: Mutex::new(()),
        }
    }

    fn lookup(&self, key: &Key) -> Option<Arc<GatherCatalog>> {
        self.slots
            .lock()
            .iter()
            .find(|slot| slot.key == *key)
            .and_then(|slot| slot.catalog.upgrade())
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
        let catalog = Arc::new(load()?);
        let mut slots = self.slots.lock();
        slots.retain(|slot| slot.key != key && slot.catalog.strong_count() > 0);
        slots.push(Slot {
            key,
            catalog: Arc::downgrade(&catalog),
        });
        Ok(catalog)
    }
}

static CACHE: Cache = Cache::new();

/// Verify and decode the selected revision's gathering family, or share the copy another user already holds.
/// Off-pump only: the `FamilyPreparation` capability exists solely on the preparation worker.
pub fn prepare(
    data: &SelectedGameData,
    _worker: &mut FamilyPreparation,
) -> Result<Arc<GatherCatalog>, FactError> {
    let pin = data.selected_pin()?;
    CACHE.prepare(Key::of(&pin), || {
        load(&pin, MANIFEST, embedded(pin.revision))
    })
}

/// The prepared catalog if some user still holds it. Never decodes and never waits for a preparation.
pub fn cached(data: &SelectedGameData) -> Option<Arc<GatherCatalog>> {
    let pin = data.selected_pin().ok()?;
    CACHE.lookup(&Key::of(&pin))
}

/// Cache-only placement lookup. The temporary Arc cannot keep a catalog alive
/// after this query; navigation never prepares or retains the gathering family.
pub fn loc_footprint_at(pin: &SelectedPin, origin: crate::snapshot::WorldTile) -> Option<(u8, u8)> {
    CACHE.lookup(&Key::of(pin))?.loc_footprint_at(origin)
}

/// Admit `family` against `manifest` and the pin, then decode it. Split from `prepare` so refusals are testable
/// with tampered bytes and so both checks are one step that runs once per prepared catalog.
fn load(
    pin: &Arc<SelectedPin>,
    manifest: &[u8],
    family: &[u8],
) -> Result<GatherCatalog, FactError> {
    family_asset::verify_family(pin, manifest, &GATHERING_FAMILY, wire::SCHEMA, family)?;
    wire::decode(pin, family)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Digest;

    fn pin_289() -> Arc<SelectedPin> {
        crate::game_data::for_revision(ClientRevision::R289)
            .unwrap()
            .selected_pin()
            .unwrap()
    }

    fn real() -> GatherCatalog {
        let pin = pin_289();
        load(&pin, MANIFEST, GATHERING_289).expect("the checked 289 family loads")
    }

    #[test]
    fn a_prepared_catalog_is_shared_and_released_after_its_last_user() {
        let cache = Cache::new();
        let key = Key::of(&pin_289());
        assert!(
            cache.lookup(&key).is_none(),
            "nothing is decoded before preparation"
        );
        let first = cache.prepare(key, || Ok(real())).unwrap();
        let second = cache
            .prepare(key, || {
                panic!("a live catalog must be shared, not decoded again")
            })
            .unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert!(Arc::ptr_eq(&cache.lookup(&key).unwrap(), &first));
        drop(first);
        assert!(cache.lookup(&key).is_some(), "one user still holds it");
        drop(second);
        assert!(cache.lookup(&key).is_none(), "released after the last user");
        let again = cache.prepare(key, || Ok(real())).unwrap();
        assert_eq!(again.methods().len(), real().methods().len());
    }

    #[test]
    fn a_failed_preparation_is_not_cached() {
        let cache = Cache::new();
        let key = Key::of(&pin_289());
        assert_eq!(
            cache
                .prepare(key, || Err(FactError::PinMismatch))
                .unwrap_err(),
            FactError::PinMismatch
        );
        assert!(cache.lookup(&key).is_none());
        assert!(cache.prepare(key, || Ok(real())).is_ok());
    }

    #[test]
    fn preparation_refuses_bytes_the_manifest_does_not_describe() {
        let pin = pin_289();
        let mut changed = GATHERING_289.to_vec();
        let at = changed.len() / 2;
        changed[at] ^= 1;
        assert_eq!(
            load(&pin, MANIFEST, &changed).unwrap_err(),
            FactError::PinMismatch,
            "a flipped byte changes the digest"
        );
        assert_eq!(
            load(&pin, MANIFEST, GATHERING_274).unwrap_err(),
            FactError::PinMismatch,
            "the other revision's family is not this pin's"
        );
        let mut tampered = MANIFEST.to_vec();
        tampered.push(b' ');
        assert_eq!(
            load(&pin, &tampered, GATHERING_289).unwrap_err(),
            FactError::PinMismatch,
            "a manifest other than the pinned one is refused before its rows are read"
        );
    }

    #[test]
    fn preparation_refuses_a_schema_other_than_the_decoders() {
        let pin = pin_289();
        let text = String::from_utf8(MANIFEST.to_vec()).unwrap();
        let mut manifest: serde_json::Value = serde_json::from_str(&text).unwrap();
        let row = manifest["revisions"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|row| row["revision"] == 289)
            .unwrap();
        row["families"]["gathering"]["schema"] = serde_json::json!(wire::SCHEMA + 1);
        let bytes = serde_json::to_vec(&manifest).unwrap();
        // The pin binds one manifest digest, so a rewritten manifest is a pin mismatch; attach a pin for it.
        let rebound = Arc::new(SelectedPin {
            manifest_sha256: sha2::Sha256::digest(&bytes).into(),
            ..(*pin).clone()
        });
        assert_eq!(
            load(&rebound, &bytes, GATHERING_289).unwrap_err(),
            FactError::Schema {
                expected: wire::SCHEMA,
                actual: wire::SCHEMA + 1
            }
        );
    }

    #[test]
    fn a_family_of_another_identity_is_refused_even_when_the_manifest_vouches_for_it() {
        let pin = pin_289();
        let text = std::str::from_utf8(GATHERING_289).unwrap();
        // Same bytes except one pinned identity: the manifest is re-issued to describe them, so only the
        // family's own header can catch it.
        let engine = &*pin.engine_commit;
        let forged = text.replacen(engine, &"0".repeat(engine.len()), 1);
        assert_ne!(forged, text);
        let mut manifest: serde_json::Value = serde_json::from_slice(MANIFEST).unwrap();
        let row = manifest["revisions"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|row| row["revision"] == 289)
            .unwrap();
        row["families"]["gathering"]["bytes"] = serde_json::json!(forged.len());
        row["families"]["gathering"]["sha256"] =
            serde_json::json!(format!("{:x}", sha2::Sha256::digest(forged.as_bytes())));
        let bytes = serde_json::to_vec(&manifest).unwrap();
        let rebound = Arc::new(SelectedPin {
            manifest_sha256: sha2::Sha256::digest(&bytes).into(),
            ..(*pin).clone()
        });
        assert_eq!(
            load(&rebound, &bytes, forged.as_bytes()).unwrap_err(),
            FactError::PinMismatch
        );
    }
}
