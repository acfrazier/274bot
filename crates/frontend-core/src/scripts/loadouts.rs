//! The operator's loadout list as the panel and TUI editors change it, the
//! form over it that the loadout rows of the command seam act on, and the
//! worn-slot vocabulary those editors lay out.

use std::sync::{Arc, LazyLock};

use api::loadout::{Loadout, WORN_SLOTS};
use host_play::local_state::LocalStateService;

/// A worn equipment slot as the editors lay it out and name it. [`Self::id`]
/// is the slot's identity in a loadout (`api::loadout::WORN_SLOTS`, the key
/// of `Loadout::worn` and the wearpos name the item search takes); the grid
/// ([`Self::layout`]) and the names ([`Self::label`]) are what the panel and
/// the TUI share, `script`'s table read as slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum WornSlot {
    /// Helmets and hats.
    Hat,
    /// Capes.
    Back,
    /// Amulets.
    Front,
    /// The weapon hand.
    RightHand,
    /// Body armour.
    Torso,
    /// The shield hand.
    LeftHand,
    /// Leg armour.
    Legs,
    /// Gloves.
    Hands,
    /// Boots.
    Feet,
    /// Rings.
    Ring,
    /// Arrows and bolts.
    Quiver,
}

impl WornSlot {
    /// Every slot, in `api::loadout::WORN_SLOTS` order.
    pub const ALL: [WornSlot; 11] = [
        Self::Hat,
        Self::Back,
        Self::Front,
        Self::RightHand,
        Self::Torso,
        Self::LeftHand,
        Self::Legs,
        Self::Hands,
        Self::Feet,
        Self::Ring,
        Self::Quiver,
    ];

    /// The compact equipment grid the editors draw, five rows of three
    /// columns; `None` is a spacer. `script`'s grid
    /// ([`script::WORN_SLOT_LAYOUT`]), read as slots once.
    pub fn layout() -> &'static [[Option<WornSlot>; 3]; 5] {
        static LAYOUT: LazyLock<[[Option<WornSlot>; 3]; 5]> = LazyLock::new(|| {
            script::WORN_SLOT_LAYOUT.map(|row| row.map(|id| id.and_then(WornSlot::of_id)))
        });
        &LAYOUT
    }

    /// The slot with the loadout identifier `id`.
    fn of_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|slot| slot.id() == id)
    }

    /// The slot's identifier in a loadout.
    pub fn id(self) -> &'static str {
        WORN_SLOTS[self as usize]
    }

    /// What the editors call the slot: `script`'s name for it.
    pub fn label(self) -> &'static str {
        script::worn_slot_label(self.id())
    }
}

/// Why the loadout store could not be reloaded or saved: the store's own
/// sentence, which the editors show after "Save failed: ", under the kind of
/// refusal it was.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LoadoutsError {
    /// The rows have nowhere to load from or save to (a list that only
    /// feeds a setting's options, a test).
    #[error("{0}")]
    NotBound(String),
    /// The store could not be read.
    #[error("{0}")]
    Load(String),
    /// The last load failed and still stands, so a save would write over
    /// rows the form never saw: nothing was saved.
    #[error("{0}")]
    NotLoaded(String),
    /// Another writer changed the store after the rows loaded: the rest of
    /// the save was not written.
    #[error("{0}")]
    Conflict(String),
    /// The store failed or refused the write, or did not settle it in time.
    #[error("{0}")]
    Write(String),
}

/// What `script::LoadoutsStore` says when it refuses, which it reports as
/// text. The kind of a refusal is read off these here and nowhere else; the
/// tests pin each against the real store.
const STORE_NOT_BOUND: &str = "loadouts are not bound to the local-state service";
const STORE_NOT_LOADED: &str = "the loadouts did not load (";
const STORE_CONFLICT: &str = "; the loadouts changed since this list was loaded";

impl LoadoutsError {
    /// The refusal of a reload (which only the test fixtures make).
    #[cfg(any(test, feature = "test-support"))]
    fn of_reload(sentence: String) -> Self {
        if sentence == STORE_NOT_BOUND {
            Self::NotBound(sentence)
        } else {
            Self::Load(sentence)
        }
    }

    /// The refusal of a save.
    fn of_save(sentence: String) -> Self {
        if sentence == STORE_NOT_BOUND {
            Self::NotBound(sentence)
        } else if sentence.starts_with(STORE_NOT_LOADED) {
            Self::NotLoaded(sentence)
        } else if sentence.contains(STORE_CONFLICT) {
            Self::Conflict(sentence)
        } else {
            Self::Write(sentence)
        }
    }
}

/// The operator's loadout rows: loaded from the operator store, edited in
/// place and written back by the loadout rows of the command seam. A front
/// end reads them here; the form below is what it lends to
/// [`crate::OperatorSession::submit`].
#[derive(Default)]
pub struct Loadouts(Box<script::LoadoutsStore>);

impl Loadouts {
    /// The rows of the operator loadout store of `service`, loaded now. A
    /// failed load leaves the list empty and keeps the reason
    /// ([`Self::load_error`]); saves then refuse.
    pub fn operator(service: &Arc<LocalStateService>) -> Self {
        Self(Box::new(script::LoadoutsStore::over(Arc::new(
            service.operator_loadouts(),
        ))))
    }

    /// The rows of the operator loadout store of `core`'s bound process
    /// state (see [`Self::operator`]).
    pub fn operator_of<Io>(core: &crate::OperatorSession<Io>) -> Self {
        let service = core
            .local_state()
            .expect("the session's local state is bound before its loadouts open");
        Self::operator(service)
    }

    /// Rows with nowhere to save them (a setting's option list, a test):
    /// saving refuses.
    pub fn detached(loadouts: Vec<Loadout>) -> Self {
        Self(Box::new(script::LoadoutsStore::detached(loadouts)))
    }

    pub(crate) fn store(&self) -> &script::LoadoutsStore {
        &self.0
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
    pub(crate) fn discard(&mut self) {
        self.0.discard();
    }

    /// Add `loadout`, or replace the row with its name.
    pub(crate) fn upsert(&mut self, loadout: Loadout) {
        self.0.upsert(loadout);
    }

    /// Replace the row at `index`, including renames; `false` (nothing
    /// changes) when `index` is out of range or the new name is taken.
    pub(crate) fn replace_at(&mut self, index: usize, loadout: Loadout) -> bool {
        self.0.replace_at(index, loadout)
    }

    /// Remove the row named `name`.
    pub(crate) fn remove(&mut self, name: &str) -> bool {
        self.0.remove(name)
    }

    /// Whether the rows hold unsaved edits.
    pub fn is_dirty(&self) -> bool {
        self.0.is_dirty()
    }

    /// Write the rows' edits through the store.
    pub(crate) fn save(&mut self) -> Result<(), LoadoutsError> {
        self.0.save().map_err(LoadoutsError::of_save)
    }
}

/// Fixtures for tests that need rows another writer, or an earlier session,
/// already wrote: the rows' edits and their write, which the product only
/// makes through the loadout rows of the command seam.
#[cfg(any(test, feature = "test-support"))]
impl Loadouts {
    /// Add `loadout`, or replace the row with its name.
    pub fn upsert_for_test(&mut self, loadout: Loadout) {
        self.upsert(loadout);
    }

    /// Remove the row named `name`.
    pub fn remove_for_test(&mut self, name: &str) -> bool {
        self.remove(name)
    }

    /// Write the rows' edits through the store.
    pub fn save_for_test(&mut self) -> Result<(), LoadoutsError> {
        self.save()
    }

    /// Read the rows again from the store, dropping unsaved edits.
    pub fn reload_for_test(&mut self) -> Result<(), LoadoutsError> {
        self.0.reload().map_err(LoadoutsError::of_reload)
    }
}

#[cfg(test)]
impl Loadouts {
    /// The rows of `backing`, loaded now.
    pub(crate) fn over_backing_for_test(backing: Arc<dyn script::LoadoutsBacking>) -> Self {
        Self(Box::new(script::LoadoutsStore::over(backing)))
    }
}

/// Why a loadout row left the form unchanged or rolled it back.
#[derive(Debug)]
pub(crate) enum Refused {
    /// There is no draft for the row to commit.
    NoDraft,
    /// No loadout is selected for the row to act on.
    NothingSelected,
    /// The draft cannot be saved (a blank name, a name another row has):
    /// nothing was written, and the status line says why.
    Invalid,
    /// The write failed: its edits were dropped, the draft is kept.
    WriteFailed(LoadoutsError),
}

const SAVED: &str = "Saved.";
const NO_LOADOUT: &str = "No loadout selected.";
const NAME_REQUIRED: &str = "Name is required.";

/// The loadout window's model, shared by the panel's Loadouts window and
/// the TUI's popup: the operator's rows, which row is selected, the working
/// copy of that row (the draft) and the line saying what the last row did.
///
/// The loadout rows of the command seam ([`crate::OperatorCommand`]'s
/// `SelectLoadout`, `EditLoadoutDraft`, `CopyEquipmentToLoadout`,
/// `CreateLoadout`, `DuplicateLoadout`, `SaveLoadout` and `DeleteLoadout`)
/// are the only way to change the rows or the draft's relation to them: a
/// front end lends the form to [`crate::OperatorSession::submit`] through
/// [`crate::Models`]. It may stage the inputs a row commits: the selection
/// ([`Self::stage_selection`]), edits of the draft ([`Self::draft_mut`]) and
/// a whole draft ([`Self::stage_draft`]); staging writes nothing.
#[derive(Default)]
pub struct LoadoutForm {
    store: Loadouts,
    /// The selected row's index in the store's rows.
    sel: usize,
    /// The working copy of the selected row. The store changes only when a
    /// row saves it.
    draft: Option<Loadout>,
    /// What the last row did, or why it did nothing.
    status: String,
    /// Moves whenever a row replaces the draft with a row's content.
    generation: u64,
}

impl LoadoutForm {
    /// A form over `store`, with nothing selected. A refused load is the
    /// status until a row replaces it.
    pub fn over(store: Loadouts) -> Self {
        let status = store
            .load_error()
            .map(|error| format!("Could not read the loadouts: {error}"))
            .unwrap_or_default();
        Self {
            store,
            status,
            ..Self::default()
        }
    }

    /// A form over the operator loadouts of `core`'s bound process state
    /// (see [`Loadouts::operator`]).
    pub fn operator_of<Io>(core: &crate::OperatorSession<Io>) -> Self {
        Self::over(Loadouts::operator_of(core))
    }

    /// The rows, for what only reads them (the options of a setting).
    pub fn store(&self) -> &Loadouts {
        &self.store
    }

    /// The rows, in store order.
    pub fn loadouts(&self) -> &[Loadout] {
        self.store.loadouts()
    }

    /// The row names, in store order.
    pub fn names(&self) -> Vec<String> {
        self.store.names()
    }

    /// The row named `name`.
    pub fn get(&self, name: &str) -> Option<&Loadout> {
        self.store.get(name)
    }

    /// Why the last load failed, while it stands.
    pub fn load_error(&self) -> Option<&str> {
        self.store.load_error()
    }

    /// The selected row's index. The selection names no row while it is at
    /// or past the number of rows.
    pub fn selected(&self) -> usize {
        self.sel
    }

    /// The selected row as the store holds it.
    pub fn selected_row(&self) -> Option<&Loadout> {
        self.store.loadouts().get(self.sel)
    }

    /// The working copy of the selected row, as edited.
    pub fn draft(&self) -> Option<&Loadout> {
        self.draft.as_ref()
    }

    /// What the last row did, or why it did nothing; empty when no row has
    /// said anything since an edit.
    pub fn status(&self) -> &str {
        &self.status
    }

    /// Moves whenever a row replaces the draft with a row's content (a
    /// selection, a save, a copy of equipment): a front end that derived
    /// text from the draft derives it again.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Stage the selection a `SelectLoadout` makes the draft from.
    pub fn stage_selection(&mut self, index: usize) {
        self.sel = index;
    }

    /// The draft, for a widget to edit in place; what a `EditLoadoutDraft`
    /// then commits. `None` while nothing is selected.
    pub fn draft_mut(&mut self) -> Option<&mut Loadout> {
        self.draft.as_mut()
    }

    /// Stage `loadout` as the whole draft, for a front end that edits the
    /// row as text and parses it when the operator commits: what a
    /// `SaveLoadout` writes.
    pub fn stage_draft(&mut self, loadout: Loadout) {
        self.draft = Some(loadout);
    }

    fn set_status(&mut self, text: &str) {
        self.status.clear();
        self.status.push_str(text);
    }

    fn selected_name(&self) -> Option<String> {
        self.selected_row().map(|row| row.name.clone())
    }

    fn position_of(&self, name: &str) -> Option<usize> {
        self.store
            .loadouts()
            .iter()
            .position(|row| row.name == name)
    }

    fn no_draft(&mut self) -> Refused {
        self.set_status(NO_LOADOUT);
        Refused::NoDraft
    }

    /// `SelectLoadout`: the draft becomes a copy of the selected row. Edits
    /// of the previous draft that were not saved are dropped.
    pub(crate) fn select(&mut self) -> Result<(), Refused> {
        self.draft = self.store.loadouts().get(self.sel).cloned();
        self.generation = self.generation.wrapping_add(1);
        match self.draft {
            Some(_) => Ok(()),
            None => Err(Refused::NothingSelected),
        }
    }

    /// `EditLoadoutDraft`: the staged edit of the draft stands, and a
    /// status about the draft's last save or copy no longer describes it.
    pub(crate) fn edited(&mut self) -> Result<(), Refused> {
        if self.draft.is_none() {
            return Err(self.no_draft());
        }
        self.status.clear();
        Ok(())
    }

    /// `CopyEquipmentToLoadout`: the worn gear of `items` (item name and
    /// id) replaces the draft's, its supplies kept. Nothing is written.
    pub(crate) fn copy_equipment(
        &mut self,
        items: &[(String, i32)],
        data: Option<&api::game_data::SelectedGameData>,
    ) -> Result<(), Refused> {
        let Some(draft) = self.draft.as_mut() else {
            return Err(self.no_draft());
        };
        script::copy_equipment_preserving_supplies(draft, items, data);
        self.generation = self.generation.wrapping_add(1);
        self.set_status("Copied current equipment into the draft. Save to keep it.");
        Ok(())
    }

    /// Say that the copy has nobody to take equipment from.
    pub(crate) fn note_no_equipment(&mut self) {
        self.set_status("Copy current equipment needs a focused ingame character.");
    }

    /// `CreateLoadout`: an empty loadout with a name no row has is written
    /// at once and selected. The rows are rolled back if the write fails.
    pub(crate) fn create(&mut self) -> Result<(), Refused> {
        let name = script::unique_loadout_name(self.store.loadouts(), "loadout");
        let selected = self.selected_name();
        self.store.upsert(Loadout::new(name.clone()));
        self.written(selected, Some(&name))
    }

    /// `DuplicateLoadout`: the selected row, as the store holds it, is
    /// copied under a name no row has, written at once and selected. The
    /// rows are rolled back if the write fails.
    pub(crate) fn duplicate(&mut self) -> Result<(), Refused> {
        let Some(source) = self.selected_row() else {
            return Err(Refused::NothingSelected);
        };
        let selected = source.name.clone();
        let copy = Loadout {
            name: script::unique_loadout_name(self.store.loadouts(), &format!("{selected} copy")),
            ..source.clone()
        };
        let name = copy.name.clone();
        self.store.upsert(copy);
        self.written(Some(selected), Some(&name))
    }

    /// `SaveLoadout`: the draft replaces the selected row (a rename keeps
    /// the row's place; a draft with no row to replace is added), written
    /// at once and selected. The rows are rolled back if the write fails;
    /// the draft is kept either way.
    pub(crate) fn save_draft(&mut self) -> Result<(), Refused> {
        let Some(draft) = self.draft.clone() else {
            return Err(self.no_draft());
        };
        if draft.name.trim().is_empty() {
            self.set_status(NAME_REQUIRED);
            return Err(Refused::Invalid);
        }
        let selected = self.selected_name();
        let saved = draft.name.clone();
        if self.sel < self.store.loadouts().len() {
            // The name is the row's key in the store: a rename onto another
            // row's name is refused rather than replacing that row.
            if !self.store.replace_at(self.sel, draft) {
                self.set_status(&format!("A loadout named \"{saved}\" already exists."));
                return Err(Refused::Invalid);
            }
        } else {
            self.store.upsert(draft);
        }
        self.written(selected, Some(&saved))
    }

    /// `DeleteLoadout`: the selected row is removed and the removal written
    /// at once; the selection stays at its index, or on the last row. The
    /// rows are rolled back if the write fails.
    pub(crate) fn delete(&mut self) -> Result<(), Refused> {
        let Some(name) = self.selected_name() else {
            return Err(Refused::NothingSelected);
        };
        self.store.remove(&name);
        self.written(Some(name), None)
    }

    /// Write the edits the row made in the store. On success the row named
    /// `select` (a saved or new row; the clamped selection after a removal)
    /// is selected and made the draft. On failure the edits are dropped, the
    /// selection returns to the row `selected` named (found again by name:
    /// after a conflict the rows are the store's latest, in an order that
    /// may differ) and the draft is kept.
    fn written(&mut self, selected: Option<String>, select: Option<&str>) -> Result<(), Refused> {
        match self.store.save() {
            Ok(()) => {
                let last = self.store.loadouts().len().saturating_sub(1);
                self.sel = match select {
                    Some(name) => self.position_of(name).unwrap_or(last),
                    None => self.sel.min(last),
                };
                // Whatever the selection names is now the draft (none when
                // the last row was removed).
                let _ = self.select();
                self.set_status(SAVED);
                Ok(())
            }
            Err(error) => {
                self.store.discard();
                if let Some(index) = selected.and_then(|name| self.position_of(&name)) {
                    self.sel = index;
                }
                self.set_status(&format!("Save failed: {error}"));
                Err(Refused::WriteFailed(error))
            }
        }
    }
}

/// Fixtures for tests that need the form's rows to be what another writer
/// wrote.
#[cfg(any(test, feature = "test-support"))]
impl LoadoutForm {
    /// Read the rows again from the store, dropping unsaved edits; the
    /// selection and the draft stay as they are.
    pub fn reload_for_test(&mut self) -> Result<(), LoadoutsError> {
        self.store.reload_for_test()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worn_slots_follow_the_loadout_slot_identifiers() {
        let ids = WornSlot::ALL.map(WornSlot::id);
        assert_eq!(ids, WORN_SLOTS);
        assert_eq!(
            ids,
            [
                "hat",
                "back",
                "front",
                "righthand",
                "torso",
                "lefthand",
                "legs",
                "hands",
                "feet",
                "ring",
                "quiver"
            ]
        );
    }

    #[test]
    fn worn_slot_labels_are_what_the_editors_always_showed() {
        let labels = WornSlot::ALL.map(WornSlot::label);
        assert_eq!(
            labels,
            [
                "Hat", "Cape", "Amulet", "Weapon", "Body", "Shield", "Legs", "Hands", "Feet",
                "Ring", "Quiver"
            ]
        );
    }

    #[test]
    fn the_layout_places_every_slot_exactly_once() {
        let placed: Vec<WornSlot> = WornSlot::layout()
            .iter()
            .flatten()
            .filter_map(|cell| *cell)
            .collect();
        assert_eq!(placed.len(), WornSlot::ALL.len());
        for slot in WornSlot::ALL {
            assert_eq!(
                placed.iter().filter(|placed| **placed == slot).count(),
                1,
                "{slot:?}"
            );
        }
        assert_eq!(WornSlot::layout()[0], [None, Some(WornSlot::Hat), None]);
    }
}
