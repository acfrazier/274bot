//! Process-shared cache for the prepared identity-only quest family.
//!
//! Preparation verifies the selected `quests.json` artifact off-pump. The
//! cache-only accessors never decode or hash the artifact.

use super::catalog::QuestCatalog;
use super::wire;
use crate::family_asset;
use crate::game_data::{QuestFamilyIdentity, SelectedGameData, MANIFEST};
use crate::selected::{ClientRevision, FactError, FamilyPreparation, SelectedPin, QUESTS_FAMILY};
use parking_lot::Mutex;
use std::num::NonZeroU16;
use std::sync::Arc;

const QUESTS_274: &[u8] = include_bytes!("../../data/game-data/274/quests.json");
const QUESTS_289: &[u8] = include_bytes!("../../data/game-data/289/quests.json");

fn embedded(revision: ClientRevision) -> &'static [u8] {
    match revision {
        ClientRevision::R274 => QUESTS_274,
        ClientRevision::R289 => QUESTS_289,
    }
}

/// One catalog is valid for one revision and one selected manifest.
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
    catalog: Arc<QuestCatalog>,
    identity: QuestFamilyIdentity,
}

/// Slots hold only the two pinned revisions. Lookup locks never wait for decode.
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

    fn lookup(&self, key: &Key) -> Option<Arc<QuestCatalog>> {
        self.slots
            .lock()
            .iter()
            .flatten()
            .find(|slot| slot.key == *key)
            .map(|slot| Arc::clone(&slot.catalog))
    }

    fn lookup_identity(&self, key: &Key) -> Option<QuestFamilyIdentity> {
        self.slots
            .lock()
            .iter()
            .flatten()
            .find(|slot| slot.key == *key)
            .map(|slot| slot.identity)
    }

    fn prepare(
        &self,
        key: Key,
        load: impl FnOnce() -> Result<(QuestCatalog, QuestFamilyIdentity), FactError>,
    ) -> Result<Arc<QuestCatalog>, FactError> {
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
        let (catalog, identity) = load()?;
        let catalog = Arc::new(catalog);
        self.slots.lock()[empty] = Some(Slot {
            key,
            catalog: Arc::clone(&catalog),
            identity,
        });
        Ok(catalog)
    }
}

static CACHE: Cache = Cache::new();

/// Verify and prepare the selected identity-only quest family, or share its catalog.
/// The `FamilyPreparation` capability confines this operation to off-pump preparation.
pub(crate) fn prepare(
    data: &SelectedGameData,
    _worker: &mut FamilyPreparation,
) -> Result<Arc<QuestCatalog>, FactError> {
    let pin = data.selected_pin()?;
    CACHE.prepare(Key::of(&pin), || {
        load(&pin, MANIFEST, embedded(pin.revision))
    })
}

/// Return a prepared catalog without decoding or waiting for a preparation.
pub(crate) fn cached(data: &SelectedGameData) -> Option<Arc<QuestCatalog>> {
    let pin = data.selected_pin().ok()?;
    CACHE.lookup(&Key::of(&pin))
}

/// Return the already-verified artifact identity without decoding or hashing.
pub(crate) fn cached_identity(data: &SelectedGameData) -> Option<QuestFamilyIdentity> {
    let pin = data.selected_pin().ok()?;
    CACHE.lookup_identity(&Key::of(&pin))
}

fn load(
    pin: &SelectedPin,
    manifest: &[u8],
    family: &[u8],
) -> Result<(QuestCatalog, QuestFamilyIdentity), FactError> {
    let (facts, identity) = verified_identity(pin, manifest, family)?;
    let catalog = QuestCatalog::from_identity(Some(&facts))?;
    Ok((catalog, identity))
}

/// The offline bake consumes identity rows, not a runtime progress catalog.
/// Keep verification shared with normal worker preparation while avoiding
/// construction of a catalog that the bake would immediately discard.
pub(crate) fn for_bake(
    data: &SelectedGameData,
) -> Result<(crate::game_data::QuestIdentityFacts, QuestFamilyIdentity), FactError> {
    let pin = data.selected_pin()?;
    verified_identity(&pin, MANIFEST, embedded(pin.revision))
}

fn verified_identity(
    pin: &SelectedPin,
    manifest: &[u8],
    family: &[u8],
) -> Result<(crate::game_data::QuestIdentityFacts, QuestFamilyIdentity), FactError> {
    let verified =
        family_asset::verify_family(pin, manifest, &QUESTS_FAMILY, wire::SCHEMA, family)?;
    let schema = NonZeroU16::new(verified.schema).ok_or(FactError::Schema {
        expected: wire::SCHEMA,
        actual: verified.schema,
    })?;
    let facts = wire::decode(pin, family)?;
    Ok((
        facts,
        QuestFamilyIdentity {
            sha256: verified.sha256,
            schema,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::super::catalog::QuestKind;
    use super::*;
    use crate::selected::Knowledge;
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};

    fn pin(revision: ClientRevision) -> Arc<SelectedPin> {
        crate::game_data::for_revision(revision)
            .unwrap()
            .selected_pin()
            .unwrap()
    }

    fn rebind_manifest(pin: &SelectedPin, manifest: &[u8]) -> Arc<SelectedPin> {
        Arc::new(SelectedPin {
            manifest_sha256: Sha256::digest(manifest).into(),
            ..(*pin).clone()
        })
    }

    fn revise_family(
        pin: &SelectedPin,
        family: &[u8],
        update: impl FnOnce(&mut Value),
    ) -> (Vec<u8>, Arc<SelectedPin>) {
        let mut manifest: Value = serde_json::from_slice(MANIFEST).unwrap();
        let row = manifest["revisions"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|row| row["revision"] == pin.revision.as_i32())
            .unwrap();
        let descriptor = &mut row["families"]["quests"];
        descriptor["bytes"] = json!(family.len());
        descriptor["sha256"] = json!(format!("{:x}", Sha256::digest(family)));
        update(descriptor);
        let manifest = serde_json::to_vec(&manifest).unwrap();
        let rebound = rebind_manifest(pin, &manifest);
        (manifest, rebound)
    }

    #[test]
    fn selected_quests_prepare_identity_only_catalogs_for_both_revisions() {
        FamilyPreparation::run(|worker| {
            for revision in [ClientRevision::R274, ClientRevision::R289] {
                let data = crate::game_data::for_revision(revision).unwrap();
                let catalog = data.prepare_quests(worker).unwrap();
                let cached = data.try_quests().expect("preparation caches its catalog");
                assert!(Arc::ptr_eq(&catalog, &cached));
                assert_eq!(
                    catalog.quest("cook").unwrap().display.as_ref(),
                    "Cook's Assistant"
                );
                assert!(matches!(
                    &catalog.quest("cook").unwrap().stages,
                    Knowledge::Unknown(gap) if &*gap.code == "identity-only"
                ));
                assert_eq!(catalog.quest("cook").unwrap().kind, QuestKind::Quest);

                let identity = data
                    .quest_family_identity()
                    .expect("verified artifact identity is cached");
                let bytes = embedded(revision);
                let actual_sha256: [u8; 32] = Sha256::digest(bytes).into();
                assert_eq!(identity.sha256, actual_sha256);
                assert_eq!(identity.schema, NonZeroU16::new(wire::SCHEMA).unwrap());

                match revision {
                    ClientRevision::R274 => {
                        assert!(matches!(
                            catalog.quest("routequest"),
                            Err(FactError::UnknownKey(_))
                        ));
                        assert!(catalog.identity_coverage().iter().any(|fact| {
                            fact.alias == "routequest" && fact.on_revision == 274 && !fact.copied
                        }));
                    }
                    ClientRevision::R289 => {
                        assert!(catalog.quest("routequest").is_ok());
                        assert!(catalog.identity_coverage().is_empty());
                    }
                }
            }
        })
        .unwrap()
        .join()
        .unwrap();
    }

    #[test]
    fn preparation_refuses_digest_path_and_selected_pin_mismatches() {
        let pin = pin(ClientRevision::R289);
        let mut changed = QUESTS_289.to_vec();
        let at = changed.len() / 2;
        changed[at] ^= 1;
        assert_eq!(
            load(&pin, MANIFEST, &changed).err().unwrap(),
            FactError::PinMismatch
        );
        assert_eq!(
            load(&pin, MANIFEST, QUESTS_274).err().unwrap(),
            FactError::PinMismatch
        );
        let mut changed_manifest = MANIFEST.to_vec();
        changed_manifest.push(b' ');
        assert_eq!(
            load(&pin, &changed_manifest, QUESTS_289).err().unwrap(),
            FactError::PinMismatch
        );

        let (manifest, rebound) = revise_family(&pin, QUESTS_289, |descriptor| {
            descriptor["path"] = json!("other/quests.json");
        });
        assert_eq!(
            load(&rebound, &manifest, QUESTS_289).err().unwrap(),
            FactError::PinMismatch
        );

        let mut foreign_family: Value = serde_json::from_slice(QUESTS_289).unwrap();
        foreign_family["engine_commit"] = json!("0".repeat(pin.engine_commit.len()));
        let foreign_family = serde_json::to_vec(&foreign_family).unwrap();
        let (manifest, rebound) = revise_family(&pin, &foreign_family, |_| {});
        assert_eq!(
            load(&rebound, &manifest, &foreign_family).err().unwrap(),
            FactError::PinMismatch
        );
    }

    #[test]
    fn preparation_refuses_schema_mismatches_and_stage_payloads() {
        let pin = pin(ClientRevision::R289);
        let (manifest, rebound) = revise_family(&pin, QUESTS_289, |descriptor| {
            descriptor["schema"] = json!(wire::SCHEMA + 1);
        });
        assert_eq!(
            load(&rebound, &manifest, QUESTS_289).err().unwrap(),
            FactError::Schema {
                expected: wire::SCHEMA,
                actual: wire::SCHEMA + 1,
            }
        );

        let mut wrong_schema: Value = serde_json::from_slice(QUESTS_289).unwrap();
        wrong_schema["schema"] = json!(wire::SCHEMA + 1);
        let wrong_schema = serde_json::to_vec(&wrong_schema).unwrap();
        let (manifest, rebound) = revise_family(&pin, &wrong_schema, |_| {});
        assert_eq!(
            load(&rebound, &manifest, &wrong_schema).err().unwrap(),
            FactError::Schema {
                expected: wire::SCHEMA,
                actual: wire::SCHEMA + 1,
            }
        );

        let mut stage_payload: Value = serde_json::from_slice(QUESTS_289).unwrap();
        stage_payload["facts"]["stages"] = json!([]);
        let stage_payload = serde_json::to_vec(&stage_payload).unwrap();
        let (manifest, rebound) = revise_family(&pin, &stage_payload, |_| {});
        assert!(matches!(
            load(&rebound, &manifest, &stage_payload),
            Err(FactError::Invalid { .. })
        ));
    }
}
