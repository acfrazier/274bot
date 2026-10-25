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
    static COMBAT: RefCell<Option<Result<Arc<script::combat::tables::CombatTables>, script::native::ActionError>>> = const { RefCell::new(None) };
}
/// The actionable explanation when selected content facts were not verified.
pub(crate) const GAME_DATA_UNAVAILABLE: &str =
    "game data unavailable: this server's content isn't verified (see profile/engine settings)";

pub fn configure(data: Option<Arc<SelectedGameData>>) {
    let casket_id = api::content::random_event_casket_id(data.as_deref());
    GAME_DATA.with(|slot| *slot.borrow_mut() = data);
    GATHERING.with(|slot| slot.borrow_mut().take());
    RANDOM_EVENT_CASKET_ID.with(|slot| slot.set(casket_id));

    COMBAT.with(|slot| slot.borrow_mut().take());
}

pub(crate) fn selected_data() -> Option<Arc<SelectedGameData>> {
    GAME_DATA.with(|slot| slot.borrow().clone())
}

/// Cache both the selected combat index and an actionable build failure until
/// this isolate's selected content is reconfigured.
pub(crate) fn combat_tables(
) -> Result<Arc<script::combat::tables::CombatTables>, script::native::ActionError> {
    COMBAT.with(|slot| {
        let mut cached = slot.borrow_mut();
        cached
            .get_or_insert_with(|| {
                selected_data()
                    .ok_or_else(|| {
                        script::native::ActionError::Unavailable(GAME_DATA_UNAVAILABLE.into())
                    })
                    .and_then(script::combat::tables::CombatTables::build)
            })
            .clone()
    })
}

pub(crate) fn random_event_casket_id() -> Option<i32> {
    RANDOM_EVENT_CASKET_ID.with(Cell::get)
}

/// The selected gathering catalogue, prepared lazily on first use and
/// retained by this isolate thread until reconfiguration. `Ok(None)` means
/// the selected family is genuinely absent; a work refusal or outage is an
/// error, never an empty catalogue.
pub(crate) fn gathering() -> Result<Option<Arc<GatherCatalog>>, String> {
    if let Some(held) = GATHERING.with(|slot| slot.borrow().clone()) {
        return Ok(Some(held));
    }
    let catalog = acquire_gathering()?;
    if let Some(catalog) = &catalog {
        GATHERING.with(|slot| *slot.borrow_mut() = Some(Arc::clone(catalog)));
    }
    Ok(catalog)
}

/// The isolate is neither UI nor slot pump. On temporary Full it retains one
/// Arc input, parks for a capacity notification and retries with a fresh
/// WorkIdentity. Accepted work is joined without a fixed busy-queue timeout.
fn acquire_gathering() -> Result<Option<Arc<GatherCatalog>>, String> {
    let data = selected_data().ok_or("missing-selected-data")?;
    if let Some(catalogue) = data.try_gathering() {
        return Ok(Some(catalogue));
    }
    let revision = u64::try_from(data.revision()).map_err(|error| error.to_string())?;
    let input = script::preparation::PreparationInput::new(
        revision,
        std::mem::size_of::<Arc<SelectedGameData>>(),
        2 * 1024 * 1024,
        2 * 1024 * 1024,
    )
    .map_err(|error| format!("work-admission-refused:{error}"))?
    .with_gathering_family_decode();
    let service = script::work_admission::WorkService::shared()
        .map_err(|error| format!("work-admission-unavailable:{error}"))?;
    loop {
        let observed = service.lock().publications().capacity().sequence();
        match script::preparation::submit(
            script::preparation::PreparationKind::Profile,
            api::identity::Scope::Process,
            input,
            {
                let data = Arc::clone(&data);
                move |worker| data.prepare_gathering(worker)
            },
        ) {
            Ok(job) => {
                return match job
                    .join()
                    .map_err(|error| format!("work-admission-job:{error}"))?
                {
                    Ok(catalog) => Ok(Some(catalog)),
                    Err(api::selected::FactError::FamilyUnavailable(_)) => Ok(None),
                    Err(error) => Err(format!("gathering-family:{error:?}")),
                };
            }
            Err(script::preparation::PreparationError::Refused(
                api::work::SubmissionRefusal::Full,
            )) => script::work_admission::WorkService::wait_for_capacity_change(&service, observed),
            Err(error) => return Err(format!("work-admission-refused:{error}")),
        }
    }
}
