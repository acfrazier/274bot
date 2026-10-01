//! Own test binary (it installs a counting global allocator): the borrowed placement and resource queries
//! allocate nothing per call, so a bounded prefix costs only what the caller takes.
use api::game_data::{for_revision, SelectedGameData};
use api::gather_methods::{GatherCatalog, SceneRegionInput};
use api::selected::{ClientRevision, FamilyPreparation};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::Arc;

struct Counting;

thread_local! {
    /// Allocation and reallocation events on this thread only, so other test threads cannot disturb a count.
    static EVENTS: Cell<usize> = const { Cell::new(0) };
}

fn bump() {
    let _ = EVENTS.try_with(|events| events.set(events.get() + 1));
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        bump();
        System.alloc(layout)
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        bump();
        System.alloc_zeroed(layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        bump();
        System.realloc(ptr, layout, new_size)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn events() -> usize {
    EVENTS.with(Cell::get)
}

fn prepare(data: &Arc<SelectedGameData>) -> Arc<GatherCatalog> {
    let data = Arc::clone(data);
    FamilyPreparation::run(move |worker| data.prepare_gathering(worker))
        .expect("worker")
        .join()
        .expect("worker panicked")
        .expect("gathering family prepares")
}

fn region(min: i32, max: i32) -> SceneRegionInput {
    SceneRegionInput {
        min_x: min,
        min_z: min,
        max_x: max,
        max_z: max,
        level: 0,
    }
}

#[test]
fn borrowed_placement_and_resource_queries_allocate_nothing() {
    for revision in [ClientRevision::R289, ClientRevision::R274] {
        let data = for_revision(revision).unwrap();
        let catalog = prepare(&data);
        let method = catalog.method("woodcutting.normal").expect("method");
        let world = region(0, 16383);
        let local = region(3200, 3263);

        let pin = data.selected_pin().unwrap();
        let yew = api::snapshot::WorldTile {
            x: 3085,
            z: 3468,
            level: 0,
        };
        // Behavior first: the lazy walk still yields exactly the region's spots, ascending by id.
        let all: Vec<_> = catalog.spots(method, &world).unwrap().collect();
        assert!(all.len() > 100, "{revision:?}: world has many placements");
        assert!(all.windows(2).all(|pair| pair[0].id < pair[1].id));
        let brute: Vec<_> = all
            .iter()
            .filter(|spot| {
                (local.min_x..=local.max_x).contains(&spot.origin.x)
                    && (local.min_z..=local.max_z).contains(&spot.origin.z)
            })
            .collect();
        let near: Vec<_> = catalog.spots(method, &local).unwrap().collect();
        assert_eq!(near.len(), brute.len(), "{revision:?}");
        assert!(near.iter().zip(&brute).all(|(a, b)| std::ptr::eq(*a, **b)));
        assert!(
            catalog
                .spots(
                    method,
                    &SceneRegionInput {
                        min_x: 10,
                        max_x: 5,
                        ..world
                    }
                )
                .unwrap()
                .next()
                .is_none(),
            "an inverted box holds nothing"
        );
        assert!(catalog.methods_for_resource("  NORMAL ").next().is_some());
        assert!(catalog.methods_for_resource("   ").next().is_none());

        let live = events();
        let owned: Vec<_> = catalog.spots(method, &local).unwrap().collect();
        assert!(
            events() > live,
            "the counter sees this thread's allocations"
        );
        drop(owned);

        const CALLS: usize = 1_000;
        let mut taken = 0usize;
        let before = events();
        for _ in 0..CALLS {
            taken += catalog.spots(method, &world).unwrap().take(1).count();
        }
        let world_take_one = events() - before;

        let before = events();
        for _ in 0..CALLS {
            taken += catalog.spots(method, &local).unwrap().count();
        }
        let local_full = events() - before;

        let before = events();
        for _ in 0..CALLS {
            taken += catalog.methods_for_resource(" normal ").count();
        }
        let resource = events() - before;
        let before = events();
        for _ in 0..CALLS {
            std::hint::black_box(api::gather_methods::loc_footprint_at(&pin, yew));
        }
        let footprint = events() - before;

        assert!(taken >= CALLS * 2);
        assert_eq!(world_take_one, 0, "{revision:?} world spots(..).take(1)");
        assert_eq!(local_full, 0, "{revision:?} local spots");
        assert_eq!(resource, 0, "{revision:?} methods_for_resource");
        assert_eq!(footprint, 0, "{revision:?} cache-only loc footprint");
    }
}
