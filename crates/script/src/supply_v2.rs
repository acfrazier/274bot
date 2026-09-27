//! Selected-cache slot for supply helper facts. New v2 entrypoints marshal
//! through typed V8 in `load/supply_v8.rs`; this module is not a JSON op.

use api::game_data::SelectedGameData;
use std::cell::RefCell;
use std::sync::Arc;

thread_local! {
    static GAME_DATA: RefCell<Option<Arc<SelectedGameData>>> = const { RefCell::new(None) };
}
/// The actionable explanation when selected content facts were not verified.
pub(crate) const GAME_DATA_UNAVAILABLE: &str =
    "game data unavailable: this server's content isn't verified (see profile/engine settings)";

pub fn configure(data: Option<Arc<SelectedGameData>>) {
    GAME_DATA.with(|slot| *slot.borrow_mut() = data);
}

pub(crate) fn selected_data() -> Option<Arc<SelectedGameData>> {
    GAME_DATA.with(|slot| slot.borrow().clone())
}
