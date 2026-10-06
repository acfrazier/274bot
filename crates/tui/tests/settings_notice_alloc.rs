//! Own test binary (it installs a counting global allocator): the settings
//! popup's save notice costs a drawn frame no allocation, whatever it says.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use frontend_core::{FormNotice, MapBakeChoice};
use host_play::WalkGlobals;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;
use tui::settings::{SettingsPane, SettingsState};
use vault::ProfileSettings;

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

/// Allocation events of one draw of the open popup with `notice`, and the
/// screen it drew.
fn draw(notice: Option<&FormNotice>, area: Rect) -> (usize, Buffer) {
    let mut settings = ProfileSettings::default();
    let mut nav = WalkGlobals::default();
    let mut bake = MapBakeChoice::Ask;
    let mut state = SettingsState {
        open: true,
        ..Default::default()
    };
    let mut pane = SettingsPane::new(&mut settings, &mut nav, &mut bake, &mut state);
    pane.notice = notice;
    let mut buf = Buffer::empty(area);
    let before = events();
    pane.render(area, &mut buf);
    (events() - before, buf)
}

fn screen(buf: &Buffer) -> String {
    buf.content().iter().map(|cell| cell.symbol()).collect()
}

#[test]
fn a_notice_adds_no_allocation_to_a_drawn_frame() {
    let long = format!(
        "random: {}",
        "vault write failed: permission denied ".repeat(4)
    );
    let notices = [
        FormNotice::Refused("settings: random: no profile alice".into()),
        FormNotice::Failed("random: Permission denied (os error 13)".into()),
        FormNotice::Failed(long),
        FormNotice::Saved("Saved alice.".into()),
    ];
    for (w, h) in [(80, 24), (120, 40)] {
        let area = Rect::new(0, 0, w, h);
        let (baseline, plain) = draw(None, area);
        assert!(!screen(&plain).contains("alice"), "{w}x{h}");
        for notice in &notices {
            let (with, drawn) = draw(Some(notice), area);
            assert!(
                screen(&drawn).contains("Saved alice.") || screen(&drawn).contains("random: "),
                "{w}x{h}: the notice is drawn: {notice:?}"
            );
            assert_eq!(
                with, baseline,
                "{w}x{h}: {notice:?} costs the frame allocations"
            );
        }
    }
}
