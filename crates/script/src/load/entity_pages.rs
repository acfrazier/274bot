//! Private per-isolate pairing between materialized entity rows and native facts.

use crate::scene_query::{EntityCategory, EntityGeneration};
use rustyscript::deno_core::v8;
use rustyscript::Runtime;
use std::cell::RefCell;
use std::rc::Rc;

/// The last successful public-row/native-fact pair for each scene category.
///
/// Only the exact materialized row objects and shared native page owners live
/// here. The mutable JavaScript arrays and wrapper/query objects are not cached.
#[derive(Default)]
pub(super) struct EntityPages {
    pub generation: EntityGeneration,
    pub npcs: Vec<v8::Global<v8::Object>>,
    pub locs: Vec<v8::Global<v8::Object>>,
    pub players: Vec<v8::Global<v8::Object>>,
    pub ground: Vec<v8::Global<v8::Object>>,
}

impl EntityPages {
    pub(super) fn rows(&self, category: EntityCategory) -> &[v8::Global<v8::Object>] {
        match category {
            EntityCategory::Npcs => &self.npcs,
            EntityCategory::Players => &self.players,
            EntityCategory::Locs => &self.locs,
            EntityCategory::Ground => &self.ground,
        }
    }
}

/// Stage the changed entity categories until the whole snapshot is published.
#[derive(Default)]
pub(super) struct StagedEntityPages {
    pub(super) generation: Option<EntityGeneration>,
    pub(super) npcs: Option<Vec<v8::Global<v8::Object>>>,
    pub(super) locs: Option<Vec<v8::Global<v8::Object>>>,
    pub(super) players: Option<Vec<v8::Global<v8::Object>>>,
    pub(super) ground: Option<Vec<v8::Global<v8::Object>>>,
    pub(super) self_slot: Option<i32>,
}

impl StagedEntityPages {
    /// Publish every changed row/fact pair together after snapshot success.
    pub(super) fn commit(self, state: &Rc<RefCell<EntityPages>>) {
        let Self {
            generation,
            npcs,
            locs,
            players,
            ground,
            self_slot,
        } = self;
        if npcs.is_none()
            && locs.is_none()
            && players.is_none()
            && ground.is_none()
            && self_slot.is_none()
        {
            return;
        }

        let EntityGeneration {
            npcs: native_npcs,
            locs: native_locs,
            players: native_players,
            ground: native_ground,
            self_slot: _,
        } = generation.expect("entity page updates capture their posting generation");

        // Defer releasing old V8 globals until after the RefCell borrow ends.
        // This runs on the live isolate thread while its HandleScope is active.
        let mut retired = EntityPages::default();
        {
            let mut current = state.borrow_mut();
            if let Some(next) = npcs {
                retired.npcs = std::mem::replace(&mut current.npcs, next);
                retired.generation.npcs =
                    std::mem::replace(&mut current.generation.npcs, native_npcs);
            }
            if let Some(next) = locs {
                retired.locs = std::mem::replace(&mut current.locs, next);
                retired.generation.locs =
                    std::mem::replace(&mut current.generation.locs, native_locs);
            }
            if let Some(next) = players {
                retired.players = std::mem::replace(&mut current.players, next);
                retired.generation.players =
                    std::mem::replace(&mut current.generation.players, native_players);
            }
            if let Some(next) = ground {
                retired.ground = std::mem::replace(&mut current.ground, next);
                retired.generation.ground =
                    std::mem::replace(&mut current.generation.ground, native_ground);
            }
            if let Some(next) = self_slot {
                retired.generation.self_slot =
                    std::mem::replace(&mut current.generation.self_slot, next);
            }
        }
        drop(retired);
    }
}

/// Install an empty native mapping in this isolate's private V8 slot.
pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    let state = Rc::new(RefCell::new(EntityPages::default()));
    if runtime.deno_runtime().v8_isolate().set_slot(state) {
        Ok(())
    } else {
        Err("entity page mapping already installed".into())
    }
}

/// Read the private mapping slot without exposing it to JavaScript.
pub(super) fn state(scope: &v8::HandleScope) -> Option<Rc<RefCell<EntityPages>>> {
    scope.get_slot::<Rc<RefCell<EntityPages>>>().cloned()
}

/// Ensure standalone Runtime materialization starts from a real empty mapping.
pub(super) fn ensure(runtime: &mut Runtime) -> Result<(), String> {
    if runtime
        .deno_runtime()
        .v8_isolate()
        .get_slot::<Rc<RefCell<EntityPages>>>()
        .is_some()
    {
        Ok(())
    } else {
        install(runtime)
    }
}

/// Clear all row roots and native owners while the isolate is still alive.
pub(super) fn reset(runtime: &mut Runtime) {
    let Some(state) = runtime
        .deno_runtime()
        .v8_isolate()
        .get_slot::<Rc<RefCell<EntityPages>>>()
        .cloned()
    else {
        return;
    };
    let retired = {
        let mut current = state.borrow_mut();
        std::mem::take(&mut *current)
    };
    drop(retired);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_commit_preserves_omitted_pairs_and_capacities() {
        let state = Rc::new(RefCell::new(EntityPages::default()));
        let first_npcs = Rc::new(Vec::new());
        let first_locs = Rc::new(Vec::new());
        let npc_rows = Vec::with_capacity(5);
        let loc_rows = Vec::with_capacity(7);
        let npc_capacity = npc_rows.capacity();
        let loc_capacity = loc_rows.capacity();

        StagedEntityPages {
            generation: Some(EntityGeneration {
                npcs: Some(first_npcs.clone()),
                locs: Some(first_locs.clone()),
                self_slot: 12,
                ..EntityGeneration::default()
            }),
            npcs: Some(npc_rows),
            locs: Some(loc_rows),
            self_slot: Some(12),
            ..StagedEntityPages::default()
        }
        .commit(&state);
        {
            let pages = state.borrow();
            assert_eq!(pages.npcs.capacity(), npc_capacity);
            assert_eq!(pages.locs.capacity(), loc_capacity);
            assert!(Rc::ptr_eq(
                pages.generation.npcs.as_ref().expect("first NPC page"),
                &first_npcs
            ));
            assert!(Rc::ptr_eq(
                pages.generation.locs.as_ref().expect("first loc page"),
                &first_locs
            ));
        }

        let second_locs = Rc::new(Vec::new());
        let next_loc_rows = Vec::with_capacity(3);
        let next_loc_capacity = next_loc_rows.capacity();
        StagedEntityPages {
            generation: Some(EntityGeneration {
                locs: Some(second_locs.clone()),
                self_slot: 99,
                ..EntityGeneration::default()
            }),
            locs: Some(next_loc_rows),
            self_slot: Some(99),
            ..StagedEntityPages::default()
        }
        .commit(&state);

        let pages = state.borrow();
        assert!(Rc::ptr_eq(
            pages.generation.npcs.as_ref().expect("first NPC page"),
            &first_npcs
        ));
        assert!(Rc::ptr_eq(
            pages
                .generation
                .locs
                .as_ref()
                .expect("replacement loc page"),
            &second_locs
        ));
        assert_eq!(pages.npcs.capacity(), npc_capacity);
        assert_eq!(pages.locs.capacity(), next_loc_capacity);
        assert_eq!(pages.generation.self_slot, 99);
        assert!(!Rc::ptr_eq(
            pages
                .generation
                .locs
                .as_ref()
                .expect("replacement loc page"),
            &first_locs
        ));
    }
}
