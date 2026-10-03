//! Selected-cache slot for supply helper facts. New v2 entrypoints marshal
//! through typed V8 in `load/supply_v8.rs`; this module is not a JSON op.

use api::game_data::SelectedGameData;
use api::gather_methods::GatherCatalog;
use std::cell::{Cell, RefCell};
use std::sync::Arc;

thread_local! {
    static GAME_DATA: RefCell<Option<Arc<SelectedGameData>>> = const { RefCell::new(None) };
    /// The gathering catalog this isolate thread has read: shared with every other holder through the api's
    /// weak cache, and released when the slot is reconfigured or the thread ends.
    static GATHERING: RefCell<Option<Arc<GatherCatalog>>> = const { RefCell::new(None) };
    static RANDOM_EVENT_CASKET_ID: Cell<Option<i32>> = const { Cell::new(None) };
    #[cfg(feature = "load")]
    static COMBAT: RefCell<Option<Result<Arc<crate::combat::tables::CombatTables>, crate::native::ActionError>>> = const { RefCell::new(None) };
}
/// The actionable explanation when selected content facts were not verified.
pub(crate) const GAME_DATA_UNAVAILABLE: &str =
    "game data unavailable: this server's content isn't verified (see profile/engine settings)";

pub fn configure(data: Option<Arc<SelectedGameData>>) {
    let casket_id = api::content::random_event_casket_id(data.as_deref());
    GAME_DATA.with(|slot| *slot.borrow_mut() = data);
    GATHERING.with(|slot| slot.borrow_mut().take());
    RANDOM_EVENT_CASKET_ID.with(|slot| slot.set(casket_id));
    #[cfg(feature = "load")]
    COMBAT.with(|slot| slot.borrow_mut().take());
}

pub(crate) fn selected_data() -> Option<Arc<SelectedGameData>> {
    GAME_DATA.with(|slot| slot.borrow().clone())
}

/// Cache both the selected combat index and an actionable build failure until
/// this isolate's selected content is reconfigured.
#[cfg(feature = "load")]
pub(crate) fn combat_tables() -> Result<Arc<crate::combat::tables::CombatTables>, crate::native::ActionError> {
    COMBAT.with(|slot| {
        let mut cached = slot.borrow_mut();
        cached
            .get_or_insert_with(|| {
                selected_data()
                    .ok_or_else(|| crate::native::ActionError::Unavailable(GAME_DATA_UNAVAILABLE.into()))
                    .and_then(crate::combat::tables::CombatTables::build)
            })
            .clone()
    })
}

pub(crate) fn random_event_casket_id() -> Option<i32> {
    RANDOM_EVENT_CASKET_ID.with(Cell::get)
}

/// The selected gathering catalog, prepared lazily on first use and retained by this isolate thread until the
/// slot is reconfigured or the thread ends.
#[cfg(feature = "load")]
pub(crate) fn gathering() -> Option<Arc<GatherCatalog>> {
    if let Some(held) = GATHERING.with(|slot| slot.borrow().clone()) {
        return Some(held);
    }
    let catalog = acquire_gathering()?;
    GATHERING.with(|slot| *slot.borrow_mut() = Some(Arc::clone(&catalog)));
    Some(catalog)
}

/// A holder elsewhere makes this a cache hit; a cold cache is prepared on a preparation worker (the only place
/// the family decodes) and joined. `None` is an unavailable family: no selected data, or the embedded family
/// failed its pin, manifest or schema checks.
#[cfg(feature = "load")]
fn acquire_gathering() -> Option<Arc<GatherCatalog>> {
    let data = selected_data()?;
    data.try_gathering().or_else(|| {
        let data = Arc::clone(&data);
        api::selected::FamilyPreparation::run(move |worker| data.prepare_gathering(worker))
            .ok()?
            .join()
            .ok()?
            .ok()
    })
}
