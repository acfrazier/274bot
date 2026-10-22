//! Own test binary (it installs a counting global allocator): drawing the
//! open world picker costs a frame no allocation. Its rows are built when
//! the picker opens, from core's cached world list, never per frame.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use tui::world_picker::WorldPicker;

struct Counting;

thread_local! {
    /// Allocation and reallocation events on this thread only, so other
    /// test threads cannot disturb a count.
    static EVENTS: Cell<usize> = const { Cell::new(0) };
}

fn bump() {
    let _ = EVENTS.try_with(|events| events.set(events.get() + 1));
}

// SAFETY: every method forwards its arguments unchanged to `System`, which
// upholds the `GlobalAlloc` contract; the counter only reads and writes a
// thread-local cell and never allocates.
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

#[test]
fn a_drawn_world_picker_allocates_nothing() {
    for (w, h) in [(80, 24), (120, 40)] {
        let area = Rect::new(0, 0, w, h);
        for (profile, current) in [(Some("alice".to_string()), Some(2)), (None, None)] {
            let mut picker = WorldPicker::open(profile.clone(), current, 1..=30u16);
            let mut buf = Buffer::empty(area);
            let mut hits = Vec::new();
            // The first draw sizes the list of clickable rows.
            picker.render(area, &mut buf, &mut hits);
            assert!(!hits.is_empty(), "{w}x{h}: {profile:?} draws its rows");
            for step in [0, 7, -3, 40] {
                picker.step(step);
                hits.clear();
                let before = events();
                picker.render(area, &mut buf, &mut hits);
                assert_eq!(
                    events() - before,
                    0,
                    "{w}x{h}: a frame of the picker for {profile:?} allocated"
                );
            }
            // The rows are the ones the picker was opened with.
            let screen: String = buf.content().iter().map(|cell| cell.symbol()).collect();
            assert!(screen.contains("World 30"), "{w}x{h}: {screen}");
        }
    }
}
