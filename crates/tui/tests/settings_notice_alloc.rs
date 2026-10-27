//! Own test binary (it installs a counting global allocator): the settings
//! popup's save notice costs a drawn frame no allocation, whatever it says.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use frontend_core::SettingsDraft;
use frontend_core::{FormNotice, MapBakeChoice, NavPreference, Setting, WalkGlobals};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;
use tui::settings::{SettingsPane, SettingsState};
use tui::SettingFeedback;

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
    let settings = SettingsDraft::default();
    let mut nav = WalkGlobals::default();
    let mut bake = MapBakeChoice::Ask;
    let mut state = SettingsState {
        open: true,
        ..Default::default()
    };
    let mut pane = SettingsPane::new(&settings, &mut nav, &mut bake, &mut state);
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
        FormNotice::refused("settings: random: no profile alice"),
        FormNotice::failed("random: Permission denied (os error 13)"),
        FormNotice::failed(long),
        FormNotice::saved("Saved alice."),
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

/// Allocation events of one draw of the open popup with the saved
/// settings' `feedback` beside their rows and both dismiss lines showing,
/// with the teleport row selected.
fn draw_feedback(feedback: &[(Setting, SettingFeedback)], area: Rect) -> (usize, Buffer) {
    let settings = SettingsDraft::default();
    let mut nav = WalkGlobals::default();
    let mut bake = MapBakeChoice::Ask;
    let mut scope_ack = false;
    let mut survivable_ack = false;
    let mut state = SettingsState {
        open: true,
        row: 5,
        ..Default::default()
    };
    let mut pane = SettingsPane::new(&settings, &mut nav, &mut bake, &mut state);
    pane.script_scope_notice_ack = Some(&mut scope_ack);
    pane.survivable_routing_notice_ack = Some(&mut survivable_ack);
    pane.feedback = feedback;
    let mut buf = Buffer::empty(area);
    let before = events();
    pane.render(area, &mut buf);
    (events() - before, buf)
}

/// A saved setting that is pending or failed adds no allocation to a drawn
/// frame: its state is drawn from the retained feedback in place, after the
/// row's text and after a notice's dismiss line, however long it is. The
/// selected setting's failure is drawn to its end, or the frame says that
/// more of it is below (Down scrolls to it).
#[test]
fn a_pending_or_failed_setting_adds_no_allocation_to_a_drawn_frame() {
    let long = format!(
        "Not saved: preferences at /home/someone/.274bot/preferences.json: {}(end)",
        "the store is busy; try again later ".repeat(5)
    );
    let teleports = Setting::Nav(NavPreference::AllowTeleports);
    let bake = Setting::MapBake;
    let scope = Setting::Nav(NavPreference::ScriptScopeNoticeAck);
    let survivable = Setting::Nav(NavPreference::SurvivableRoutingNoticeAck);
    let none: Vec<(Setting, SettingFeedback)> = Vec::new();
    let pending = vec![
        (teleports, SettingFeedback::Pending),
        (bake, SettingFeedback::Pending),
        (scope, SettingFeedback::Pending),
        (survivable, SettingFeedback::Pending),
    ];
    let failed = vec![
        (
            teleports,
            SettingFeedback::Failed(long.replace("(end)", "(teleports end)")),
        ),
        (bake, SettingFeedback::Failed("Not saved: busy".into())),
        (scope, SettingFeedback::Failed(long.clone())),
        (survivable, SettingFeedback::Failed(long)),
    ];
    for (w, h) in [(80, 24), (120, 40)] {
        let area = Rect::new(0, 0, w, h);
        // The first draw of each variant warms nothing the next one needs
        // (the popup keeps no buffer), so every draw is measured.
        let (baseline, plain) = draw_feedback(&none, area);
        assert!(!screen(&plain).contains("saving…"), "{w}x{h}");
        for (label, feedback) in [("pending", &pending), ("failed", &failed)] {
            for _ in 0..3 {
                let (with, drawn) = draw_feedback(feedback, area);
                let shown = screen(&drawn);
                match label {
                    "pending" => assert!(shown.contains("saving…"), "{w}x{h}: {shown}"),
                    _ => assert!(
                        shown.contains("Not saved:")
                            && (shown.contains("(teleports end)")
                                || shown.contains("more of this setting below (Down)")),
                        "{w}x{h}: the failure is drawn to its end or says more is below: {shown}"
                    ),
                }
                assert_eq!(
                    with, baseline,
                    "{w}x{h}: {label} feedback costs the frame allocations"
                );
            }
        }
    }
}
