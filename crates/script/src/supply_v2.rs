//! Selected-cache slot for supply helper facts. New v2 entrypoints marshal
//! through typed V8 in `load/supply_v8.rs`; this module is not a JSON op.

use api::game_data::SelectedGameData;
use api::gather_methods::GatherCatalog;
use std::cell::RefCell;
use std::sync::Arc;

thread_local! {
    static GAME_DATA: RefCell<Option<Arc<SelectedGameData>>> = const { RefCell::new(None) };
    /// The gathering catalog this isolate thread has read: shared with every other holder through the api's
    /// weak cache, and released when the slot is reconfigured or the thread ends.
    static GATHERING: RefCell<Option<Arc<GatherCatalog>>> = const { RefCell::new(None) };
}
/// The actionable explanation when selected content facts were not verified.
pub(crate) const GAME_DATA_UNAVAILABLE: &str =
    "game data unavailable: this server's content isn't verified (see profile/engine settings)";

pub fn configure(data: Option<Arc<SelectedGameData>>) {
    GAME_DATA.with(|slot| *slot.borrow_mut() = data);
    GATHERING.with(|slot| slot.borrow_mut().take());
}

pub(crate) fn selected_data() -> Option<Arc<SelectedGameData>> {
    GAME_DATA.with(|slot| slot.borrow().clone())
}

/// The selected gathering catalog, prepared lazily on first use. A holder elsewhere makes this a cache hit; a
/// cold cache is prepared on a preparation worker (the only place the family decodes) and joined. `None` is an
/// unavailable family: no selected data, or the embedded family failed its pin, manifest or schema checks.
#[cfg(feature = "load")]
pub(crate) fn gathering() -> Option<Arc<GatherCatalog>> {
    if let Some(held) = GATHERING.with(|slot| slot.borrow().clone()) {
        return Some(held);
    }
    let data = selected_data()?;
    let catalog = data.try_gathering().or_else(|| {
        let data = Arc::clone(&data);
        api::selected::FamilyPreparation::run(move |worker| data.prepare_gathering(worker))
            .ok()?
            .join()
            .ok()?
            .ok()
    })?;
    GATHERING.with(|slot| *slot.borrow_mut() = Some(Arc::clone(&catalog)));
    Some(catalog)
}
