//! Process-shared bounded cache of the prepared gathering catalog, keyed by
//! the selected manifest. There are exactly two pinned revision manifests.
//!
//! Preparation (`prepare`) needs the off-pump `FamilyPreparation` capability: it verifies the
//! embedded family file against the pin before retaining its decoded catalog. The first caller
//! pays that qualified bounded unit; subsequent callers share the catalog for this process.
//! The tick/UI path (`cached`) never decodes, hashes or waits for a preparation in flight.

use super::catalog::GatherCatalog;
use super::wire;
use crate::family_asset;
use crate::game_data::{SelectedGameData, MANIFEST};
use crate::selected::{
    ClientRevision, FactError, FamilyPreparation, SelectedPin, GATHERING_FAMILY,
};
use parking_lot::Mutex;
use std::sync::Arc;

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
    catalog: Arc<GatherCatalog>,
}

/// Slots are only touched under brief locks; `preparing` serializes decodes
/// while `lookup` (the tick path) never waits behind a decode.
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

/// Verify and decode the selected revision's gathering family once per
/// process, or share the immutable copy another caller prepared.
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

/// The prepared catalog if the selected manifest was already decoded. Never decodes or waits for preparation.
pub fn cached(data: &SelectedGameData) -> Option<Arc<GatherCatalog>> {
    let pin = data.selected_pin().ok()?;
    CACHE.lookup(&Key::of(&pin))
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
    use std::time::Instant;

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
    fn a_prepared_catalog_is_shared_after_the_last_caller_drops() {
        let cache = Cache::new();
        let key = Key::of(&pin_289());
        assert!(cache.lookup(&key).is_none(), "nothing decoded yet");
        let first = cache.prepare(key, || Ok(real())).unwrap();
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
    fn selected_revision_pins_stay_isolated_and_the_cache_is_bounded() {
        let cache = Cache::new();
        let pin_274 = crate::game_data::for_revision(ClientRevision::R274)
            .unwrap()
            .selected_pin()
            .unwrap();
        let key_274 = Key::of(&pin_274);
        let pin_289 = pin_289();
        let key_289 = Key::of(&pin_289);
        let cold_274 = Instant::now();
        let catalog_274 = cache
            .prepare(key_274, || load(&pin_274, MANIFEST, GATHERING_274))
            .unwrap();
        println!(
            "pinned gathering decode 274: input={}B cold={:?}",
            GATHERING_274.len(),
            cold_274.elapsed()
        );
        let cold_289 = Instant::now();
        let catalog_289 = cache
            .prepare(key_289, || load(&pin_289, MANIFEST, GATHERING_289))
            .unwrap();
        println!(
            "pinned gathering decode 289: input={}B cold={:?}",
            GATHERING_289.len(),
            cold_289.elapsed()
        );
        assert!(!Arc::ptr_eq(&catalog_274, &catalog_289));
        drop((catalog_274, catalog_289));
        assert!(cache.lookup(&key_274).is_some());
        assert!(cache.lookup(&key_289).is_some());
        let unknown = Key {
            revision: key_289.revision,
            manifest: [0; 32],
        };
        assert!(cache.lookup(&unknown).is_none());
        assert_eq!(
            cache
                .prepare(unknown, || panic!("a third pin cannot expand the cache"))
                .unwrap_err(),
            FactError::PinMismatch
        );
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
