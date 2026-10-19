//! The operator's loadout list as the panel and TUI editors change it, and
//! the slot vocabulary those editors lay out.

use std::sync::Arc;

use api::loadout::Loadout;
use host_play::local_state::LocalStateService;

pub use script::{
    copy_equipment_preserving_supplies, unique_loadout_name, worn_slot_label, WORN_SLOT_LAYOUT,
};

/// The operator loadout form: rows loaded from the operator store, edited
/// in place, written back by [`Self::save`].
#[derive(Default)]
pub struct Loadouts(Box<script::LoadoutsStore>);

impl Loadouts {
    /// The form over the operator loadout store of `service`, loaded now.
    /// A failed load leaves the form empty and keeps the reason
    /// ([`Self::load_error`]); saves then refuse.
    pub fn operator(service: &Arc<LocalStateService>) -> Self {
        Self(Box::new(script::LoadoutsStore::over(Arc::new(
            service.operator_loadouts(),
        ))))
    }

    /// The form over the operator loadout store of `core`'s bound process
    /// state (see [`Self::operator`]).
    pub fn operator_of<Io>(core: &crate::OperatorSession<Io>) -> Self {
        let service = core
            .local_state()
            .expect("the session's local state is bound before its loadouts open");
        Self::operator(service)
    }

    /// Rows with nowhere to save them (a setting's option list, a test):
    /// [`Self::save`] refuses.
    pub fn detached(loadouts: Vec<Loadout>) -> Self {
        Self(Box::new(script::LoadoutsStore::detached(loadouts)))
    }

    /// The form over an explicit backing (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn over(backing: Arc<dyn script::LoadoutsBacking>) -> Self {
        Self(Box::new(script::LoadoutsStore::over(backing)))
    }

    pub(crate) fn store(&self) -> &script::LoadoutsStore {
        &self.0
    }

    /// Read the rows again from the store, dropping unsaved edits.
    pub fn reload(&mut self) -> Result<(), String> {
        self.0.reload()
    }

    /// Why the last load failed, while it stands.
    pub fn load_error(&self) -> Option<&str> {
        self.0.load_error()
    }

    /// The rows, in store order.
    pub fn loadouts(&self) -> &[Loadout] {
        self.0.loadouts()
    }

    /// The row names, in store order.
    pub fn names(&self) -> Vec<String> {
        self.0.names()
    }

    /// The row named `name`.
    pub fn get(&self, name: &str) -> Option<&Loadout> {
        self.0.get(name)
    }

    /// Drop unsaved edits.
    pub fn discard(&mut self) {
        self.0.discard();
    }

    /// Add `loadout`, or replace the row with its name.
    pub fn upsert(&mut self, loadout: Loadout) {
        self.0.upsert(loadout);
    }

    /// Replace the row at `index`, including renames; `false` (nothing
    /// changes) when `index` is out of range or the new name is taken.
    pub fn replace_at(&mut self, index: usize, loadout: Loadout) -> bool {
        self.0.replace_at(index, loadout)
    }

    /// Remove the row named `name`.
    pub fn remove(&mut self, name: &str) -> bool {
        self.0.remove(name)
    }

    /// Whether the form holds unsaved edits.
    pub fn is_dirty(&self) -> bool {
        self.0.is_dirty()
    }

    /// Write the form's edits through the store.
    pub fn save(&mut self) -> Result<(), String> {
        self.0.save()
    }
}
