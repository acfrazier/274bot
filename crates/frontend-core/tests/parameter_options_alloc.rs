//! Own test binary: shared preserved-label detail lookup must not allocate
//! while a renderer repeatedly asks for cached unavailable-quest labels.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use frontend_core::scripts::resolve_parameter_options;
use script::LoadoutsStore;

struct Counting;

thread_local! {
    static EVENTS: Cell<usize> = const { Cell::new(0) };
}

fn bump() {
    let _ = EVENTS.try_with(|events| events.set(events.get() + 1));
}

// SAFETY: every method forwards its arguments unchanged to `System`, which
// upholds the `GlobalAlloc` contract; the counter uses a non-allocating TLS cell.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        bump();
        // SAFETY: same layout the caller passed.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        bump();
        // SAFETY: same layout the caller passed.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        bump();
        // SAFETY: `ptr` and `layout` come from a prior allocation of ours.
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` and `layout` come from a prior allocation of ours.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn events() -> usize {
    EVENTS.with(Cell::get)
}
fn quest_options() -> frontend_core::scripts::ParameterOptions {
    let schema = (script::quester::card::CARD.schema)();
    let quests = schema
        .iter()
        .find(|field| field.id == "quests")
        .expect("Quester schema has a quest roster");
    let bag = serde_json::Map::from_iter([("quests".into(), serde_json::json!(["cook"]))]);
    let loadouts = LoadoutsStore::at(std::env::temp_dir().join(format!(
        "frontend-core-preserved-labels-{}.json",
        std::process::id()
    )));
    resolve_parameter_options(quests, &bag, &loadouts, None)
}

fn repeated_lookup_allocations(
    options: &frontend_core::scripts::ParameterOptions,
    stored: &serde_json::Value,
) -> usize {
    let before = events();
    for _ in 0..128 {
        std::hint::black_box(
            options
                .preserved_stored_labels(std::hint::black_box(stored))
                .count(),
        );
    }
    events() - before
}

#[test]
fn repeated_unavailable_label_lookups_allocate_nothing() {
    let options = quest_options();
    let stored = serde_json::json!(["hauntedmine"]);
    let labels = options.preserved_stored_labels(&stored).collect::<Vec<_>>();
    assert_eq!(labels.len(), 1);
    assert!(labels[0].contains("Haunted Mine"));

    assert_eq!(
        repeated_lookup_allocations(&options, &stored),
        0,
        "cached labels are borrowed on every repeated lookup"
    );
}

#[test]
fn repeated_lookup_without_a_preserved_value_allocates_nothing() {
    let options = quest_options();
    let stored = serde_json::json!(["cook"]);
    assert_eq!(options.preserved_stored_labels(&stored).count(), 0);

    assert_eq!(
        repeated_lookup_allocations(&options, &stored),
        0,
        "non-preserved selections need no temporary storage"
    );
}
