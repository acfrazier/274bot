//! Private per-isolate pairing between materialized entity rows and native facts.

use crate::scene::scene_query::{EntityCategory, EntityGeneration};
use rustyscript::deno_core::v8;
use rustyscript::Runtime;
use std::cell::RefCell;
use std::rc::Rc;

/// Field names shared by every scene entity row, in materialization order.
///
/// The order is the public row's own-property order: the templates create
/// these fields before the materializer writes their values, so the row
/// never grows from an empty object one store at a time.
pub(super) const COMMON_ROW_FIELDS: [&str; 23] = [
    "index",
    "id",
    "name",
    "x",
    "z",
    "level",
    "tile",
    "distance",
    "health",
    "max_health",
    "totalHealth",
    "in_combat",
    "animating",
    "actions",
    "reachable",
    "reachable_adj",
    "combat_level",
    "target_kind",
    "target_index",
    "faceEntity",
    "size",
    "nx",
    "nz",
];

/// The NPC-only fields appended when the row carries a packed footprint.
pub(super) const NPC_NETWORK_ROW_FIELDS: [&str; 2] = ["network_x", "network_z"];

/// Per-isolate templates that give entity rows their fast-property shape.
///
/// V8 creates template properties once, migrates the cached instantiation to
/// fast properties, and copies that layout into every instance. Writing the
/// actual values afterwards therefore updates existing fast fields instead of
/// adding fields to a fresh empty object, which V8 turns into a dictionary
/// once a row exceeds its incremental-store soft limit.
pub(crate) struct EntityRowShapes {
    pub(crate) common: v8::Global<v8::ObjectTemplate>,
    pub(crate) npc_network: v8::Global<v8::ObjectTemplate>,
}

impl EntityRowShapes {
    /// Build both templates on the isolate thread that will instantiate rows.
    fn create(scope: &mut v8::HandleScope) -> Result<Self, String> {
        Ok(Self {
            common: row_template(scope, COMMON_ROW_FIELDS.iter().copied())?,
            npc_network: row_template(
                scope,
                COMMON_ROW_FIELDS
                    .iter()
                    .copied()
                    .chain(NPC_NETWORK_ROW_FIELDS),
            )?,
        })
    }
}

/// One template whose own data properties are the row's complete field list.
///
/// `Template::set` installs ordinary writable, enumerable, configurable data
/// properties. The placeholder is never observed: the materializer overwrites
/// every field before the row reaches JavaScript.
fn row_template<'a>(
    scope: &mut v8::HandleScope,
    fields: impl IntoIterator<Item = &'a str>,
) -> Result<v8::Global<v8::ObjectTemplate>, String> {
    let template = v8::ObjectTemplate::new(scope);
    let placeholder: v8::Local<v8::Data> = v8::undefined(scope).into();
    for field in fields {
        let name =
            v8::String::new_from_utf8(scope, field.as_bytes(), v8::NewStringType::Internalized)
                .ok_or_else(|| "v8 string alloc failed".to_string())?;
        template.set(name.into(), placeholder);
    }
    Ok(v8::Global::new(scope, template))
}

/// The last successful public-row/native-fact pair for each scene category.
///
/// Only the exact materialized row objects, shared native page owners and the
/// per-isolate row templates live here. The mutable JavaScript arrays and
/// wrapper/query objects are not cached.
#[derive(Default)]
pub(crate) struct EntityPages {
    pub generation: EntityGeneration,
    pub npcs: Vec<v8::Global<v8::Object>>,
    pub locs: Vec<v8::Global<v8::Object>>,
    pub players: Vec<v8::Global<v8::Object>>,
    pub ground: Vec<v8::Global<v8::Object>>,
    /// `None` only until the first snapshot after install or reset.
    pub shapes: Option<EntityRowShapes>,
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
pub(crate) struct StagedEntityPages {
    pub(crate) generation: Option<EntityGeneration>,
    pub(crate) npcs: Option<Vec<v8::Global<v8::Object>>>,
    pub(crate) locs: Option<Vec<v8::Global<v8::Object>>>,
    pub(crate) players: Option<Vec<v8::Global<v8::Object>>>,
    pub(crate) ground: Option<Vec<v8::Global<v8::Object>>>,
    pub(crate) self_slot: Option<i32>,
}

impl StagedEntityPages {
    /// Publish every changed row/fact pair together after snapshot success.
    pub(crate) fn commit(self, state: &Rc<RefCell<EntityPages>>) {
        let Self {
            generation,
            npcs,
            locs,
            players,
            ground,
            self_slot,
        } = self;
        if generation.is_none()
            && npcs.is_none()
            && locs.is_none()
            && players.is_none()
            && ground.is_none()
            && self_slot.is_none()
        {
            return;
        }

        let generation = generation
            .expect("entity page or local-origin updates capture their posting generation");
        let here = generation.here;
        let EntityGeneration {
            npcs: native_npcs,
            locs: native_locs,
            players: native_players,
            ground: native_ground,
            ..
        } = generation;

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
            current.generation.here = here;
        }
        drop(retired);
    }
}

/// Install an empty native mapping in this isolate's private V8 slot.
pub(crate) fn install(runtime: &mut Runtime) -> Result<(), String> {
    let shapes = {
        let mut scope = runtime.deno_runtime().handle_scope();
        EntityRowShapes::create(&mut scope)?
    };
    let state = Rc::new(RefCell::new(EntityPages {
        shapes: Some(shapes),
        ..EntityPages::default()
    }));
    if runtime.deno_runtime().v8_isolate().set_slot(state) {
        Ok(())
    } else {
        Err("entity page mapping already installed".into())
    }
}

/// Read the private mapping slot without exposing it to JavaScript.
pub(crate) fn state(scope: &v8::HandleScope) -> Option<Rc<RefCell<EntityPages>>> {
    scope.get_slot::<Rc<RefCell<EntityPages>>>().cloned()
}

/// Ensure standalone Runtime materialization has a mapping and row templates.
///
/// A session reset keeps the installed slot but releases every cached V8
/// handle, including the templates; the next snapshot rebuilds them on this
/// same isolate thread.
pub(crate) fn ensure(runtime: &mut Runtime) -> Result<(), String> {
    let Some(state) = runtime
        .deno_runtime()
        .v8_isolate()
        .get_slot::<Rc<RefCell<EntityPages>>>()
        .cloned()
    else {
        return install(runtime);
    };
    if state.borrow().shapes.is_some() {
        return Ok(());
    }
    let shapes = {
        let mut scope = runtime.deno_runtime().handle_scope();
        EntityRowShapes::create(&mut scope)?
    };
    state.borrow_mut().shapes = Some(shapes);
    Ok(())
}

/// Clear all row roots and native owners while the isolate is still alive.
pub(crate) fn reset(runtime: &mut Runtime) {
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

    #[test]
    fn origin_only_commit_updates_network_coords_without_replacing_pages() {
        let state = Rc::new(RefCell::new(EntityPages::default()));
        let npcs = Rc::new(Vec::new());
        let players = Rc::new(Vec::new());
        let npc_rows = Vec::with_capacity(5);
        let player_rows = Vec::with_capacity(7);
        let npc_capacity = npc_rows.capacity();
        let player_capacity = player_rows.capacity();
        let old_here = crate::scene::observed::Tile {
            x: 10,
            z: 20,
            level: 0,
        };
        StagedEntityPages {
            generation: Some(EntityGeneration {
                npcs: Some(npcs.clone()),
                players: Some(players.clone()),
                here: Some(old_here),
                self_slot: 4,
                ..EntityGeneration::default()
            }),
            npcs: Some(npc_rows),
            players: Some(player_rows),
            self_slot: Some(4),
            ..StagedEntityPages::default()
        }
        .commit(&state);

        let network_here = crate::scene::observed::Tile {
            x: 12,
            z: 22,
            level: 0,
        };
        StagedEntityPages {
            generation: Some(EntityGeneration {
                here: Some(network_here),
                ..EntityGeneration::default()
            }),
            ..StagedEntityPages::default()
        }
        .commit(&state);

        let pages = state.borrow();
        assert_eq!(pages.generation.here, Some(network_here));
        assert_eq!(pages.generation.self_slot, 4);
        assert!(Rc::ptr_eq(
            pages.generation.npcs.as_ref().expect("NPC page"),
            &npcs
        ));
        assert!(Rc::ptr_eq(
            pages.generation.players.as_ref().expect("player page"),
            &players
        ));
        assert_eq!(pages.npcs.capacity(), npc_capacity);
        assert_eq!(pages.players.capacity(), player_capacity);
    }
}
