//! Process-shared cache for prepared quest catalogs, keyed by selected revision.

use super::catalog::QuestCatalog;
use super::wire;
use crate::game_data::SelectedGameData;
use crate::selected::{ClientRevision, FactError, FamilyPreparation, QUESTS_FAMILY};
use parking_lot::Mutex;
use std::sync::Arc;

#[cfg(all(feature = "embedded-game-data", not(no_local_game_data)))]
const QUESTS_274: &[u8] = include_bytes!("../../data/game-data/274/quests.json");
#[cfg(all(feature = "embedded-game-data", not(no_local_game_data)))]
const QUESTS_289: &[u8] = include_bytes!("../../data/game-data/289/quests.json");
#[cfg(all(feature = "embedded-game-data", not(no_local_game_data)))]
const QUESTS_377: &[u8] = include_bytes!("../../data/game-data/377/quests.json");

#[cfg(all(feature = "embedded-game-data", not(no_local_game_data)))]
fn embedded(revision: ClientRevision) -> Option<&'static [u8]> {
    Some(match revision {
        ClientRevision::R274 => QUESTS_274,
        ClientRevision::R289 => QUESTS_289,
        ClientRevision::R377 => QUESTS_377,
    })
}

#[cfg(not(all(feature = "embedded-game-data", not(no_local_game_data))))]
fn embedded(_revision: ClientRevision) -> Option<&'static [u8]> {
    None
}

struct Slot {
    revision: ClientRevision,
    catalog: Arc<QuestCatalog>,
}

/// One process-shared catalog slot per supported selected revision.
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

    fn slot(revision: ClientRevision) -> usize {
        match revision {
            ClientRevision::R274 => 0,
            ClientRevision::R289 => 1,
            ClientRevision::R377 => 2,
        }
    }

    fn lookup(&self, revision: ClientRevision) -> Option<Arc<QuestCatalog>> {
        self.slots.lock()[Self::slot(revision)]
            .as_ref()
            .filter(|slot| slot.revision == revision)
            .map(|slot| Arc::clone(&slot.catalog))
    }

    fn prepare(
        &self,
        revision: ClientRevision,
        load: impl FnOnce() -> Result<QuestCatalog, FactError>,
    ) -> Result<Arc<QuestCatalog>, FactError> {
        if let Some(hit) = self.lookup(revision) {
            return Ok(hit);
        }
        let _single_flight = self.preparing.lock();
        if let Some(hit) = self.lookup(revision) {
            return Ok(hit);
        }
        let catalog = Arc::new(load()?);
        self.slots.lock()[Self::slot(revision)] = Some(Slot {
            revision,
            catalog: Arc::clone(&catalog),
        });
        Ok(catalog)
    }
}

static CACHE: Cache = Cache::new();

fn unavailable() -> FactError {
    FactError::FamilyUnavailable((*QUESTS_FAMILY).clone())
}

/// Prepare or share the selected revision's quest catalog off-pump.
pub(crate) fn prepare(
    data: &SelectedGameData,
    _worker: &mut FamilyPreparation,
) -> Result<Arc<QuestCatalog>, FactError> {
    let revision = data.selected_revision().ok_or_else(unavailable)?;
    CACHE.prepare(revision, || {
        let bytes = embedded(revision).ok_or_else(unavailable)?;
        let facts = wire::decode(bytes)?;
        QuestCatalog::from_identity(Some(&facts))
    })
}

/// Return the already-prepared catalog without decoding or waiting.
pub(crate) fn cached(data: &SelectedGameData) -> Option<Arc<QuestCatalog>> {
    CACHE.lookup(data.selected_revision()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selected::{ClientRevision, Knowledge};

    #[test]
    fn selected_quest_catalogs_are_cached_per_revision() {
        let Some(data_274) = crate::game_data::test_data(ClientRevision::R274) else {
            return;
        };
        let Some(data_289) = crate::game_data::test_data(ClientRevision::R289) else {
            return;
        };
        FamilyPreparation::run(move |worker| {
            for (revision, data) in [
                (ClientRevision::R274, data_274),
                (ClientRevision::R289, data_289),
            ] {
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
                match revision {
                    ClientRevision::R274 => assert!(matches!(
                        catalog.quest("routequest"),
                        Err(FactError::UnknownKey(_))
                    )),
                    ClientRevision::R289 => assert!(catalog.quest("routequest").is_ok()),
                    ClientRevision::R377 => unreachable!("377 has its own catalog test"),
                }
            }
        })
        .unwrap()
        .join()
        .unwrap();
    }

    #[test]
    fn selected_377_quest_family_uses_its_selected_asset() {
        let Some(data) = crate::game_data::test_data(ClientRevision::R377) else {
            return;
        };
        FamilyPreparation::run(move |worker| {
            let catalog = data.prepare_quests(worker).expect("prepared 377 quests");
            assert!(
                !catalog.is_empty(),
                "the 377 quest family has identity rows"
            );
            assert!(Arc::ptr_eq(
                &catalog,
                &data.try_quests().expect("preparation caches its catalog")
            ));
        })
        .unwrap()
        .join()
        .unwrap();
    }
}
