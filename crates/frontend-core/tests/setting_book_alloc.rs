//! Own test binary: the saved-setting state's cost. The settings book is one
//! entry per [`Setting`] and does not grow with the number of writes; a
//! steady `poll_preferences` (nothing pending, nothing new durable) and the
//! per-frame status reads allocate nothing, and a retained failure's text is
//! replaced, never accumulated.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use frontend_core::command::{DrawSwitch, OperatorCommand, RasterMode};
use frontend_core::surface::{SlotAttach, SlotSurface, SpawnProfile};
use frontend_core::{
    Models, NavPreference, OperatorNotice, OperatorSession, Setting, SettingStatus, Targets,
};
use host_play::local_state::{LocalStateService, StateRoot};
use host_play::InstancePermit;

struct Counting;

thread_local! {
    static EVENTS: Cell<usize> = const { Cell::new(0) };
}

/// Live bytes over every thread: a write's request is built on the caller's
/// thread and freed on the writer's, so a per-thread balance would drift.
static LIVE: AtomicIsize = AtomicIsize::new(0);

fn note(delta: isize, event: bool) {
    LIVE.fetch_add(delta, Ordering::Relaxed);
    if event {
        let _ = EVENTS.try_with(|events| events.set(events.get() + 1));
    }
}

// SAFETY: every method forwards its arguments unchanged to `System`, which
// upholds the `GlobalAlloc` contract; the counters use non-allocating TLS
// cells.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note(layout.size() as isize, true);
        // SAFETY: same layout the caller passed.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note(layout.size() as isize, true);
        // SAFETY: same layout the caller passed.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note(new_size as isize - layout.size() as isize, true);
        // SAFETY: `ptr` and `layout` come from a prior allocation of ours.
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note(-(layout.size() as isize), false);
        // SAFETY: `ptr` and `layout` come from a prior allocation of ours.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn events() -> usize {
    EVENTS.with(Cell::get)
}

fn live() -> isize {
    LIVE.load(Ordering::Relaxed)
}

/// A surface with no devices and no models.
struct Bare;

impl SlotSurface for Bare {
    type Io = ();

    fn attach(
        &mut self,
        _name: &str,
        profile: &mut SpawnProfile<'_>,
        _retained: Option<()>,
    ) -> SlotAttach<()> {
        profile.set_raster(RasterMode::Off);
        SlotAttach::new((), None, None)
    }

    fn lifetime_reset(&mut self, _name: &str, _walk_was_armed: bool) {}

    fn released(&mut self, _name: &str) {}

    fn models(&mut self) -> Models<'_> {
        Models::none()
    }

    fn draw_changed(&mut self, _switch: DrawSwitch, _on: bool) {}
}

fn scratch(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "274bot-setting-alloc-{label}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("test directory");
    dir
}

const SETTINGS: [Setting; 8] = [
    Setting::Draw(DrawSwitch::CaptureInput),
    Setting::Nav(NavPreference::AllowTeleports),
    Setting::DangerRouting,
    Setting::SessionLog,
    Setting::MapBake,
    Setting::QuesterPaths,
    Setting::Notice(OperatorNotice::BackgroundBots),
    Setting::ProfileOrder,
];

/// A session on a scratch state root whose first preferences read has
/// completed, and the service its writes go through.
fn ready_session(label: &str) -> (OperatorSession<()>, Arc<LocalStateService>) {
    let dir = scratch(label);
    let service = LocalStateService::open_process(StateRoot::new(dir.join("state")));
    let mut session: OperatorSession<()> = OperatorSession::new(InstancePermit::SkipLock);
    session.set_spawn_workers(false);
    session.bind_local_state(Arc::clone(&service));
    let _ = session.adopt_operator_preferences();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !session.preferences().expect("bound").ready() {
        let _ = session.preferences().expect("bound").reload();
        session.poll_preferences(&mut Bare);
        assert!(
            Instant::now() < deadline,
            "the preferences never became ready"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    (session, service)
}

#[test]
fn steady_polls_and_status_reads_allocate_nothing() {
    let (mut session, _service) = ready_session("steady");
    for _ in 0..64 {
        session.poll_preferences(&mut Bare);
    }
    let before = events();
    for _ in 0..1_000 {
        session.poll_preferences(&mut Bare);
        for setting in SETTINGS {
            assert!(matches!(
                session.setting_status(setting),
                SettingStatus::Saved
            ));
            assert!(session.setting_failure(setting).is_none());
        }
        assert!(session.next_setting_expiry().is_none());
    }
    assert_eq!(events() - before, 0, "a steady poll allocated");
}

#[test]
fn repeated_failures_keep_one_bounded_entry_and_replace_their_text() {
    let (mut session, service) = ready_session("failures");
    service.set_write_fault_for_test(Arc::new(|| true));
    let mut surface = Bare;
    let setting = Setting::MapBake;
    let mut fail_once = |session: &mut OperatorSession<()>| {
        session
            .submit(
                OperatorCommand::SetMapBakeChoice(frontend_core::MapBakeChoice::Always),
                Targets::Application,
                &mut Bare,
            )
            .expect("admitted");
        let deadline = Instant::now() + Duration::from_secs(30);
        while !matches!(session.setting_status(setting), SettingStatus::Failed(_)) {
            session.poll_preferences(&mut surface);
            assert!(Instant::now() < deadline, "the write never failed");
            std::thread::sleep(Duration::from_millis(1));
        }
    };
    // Warm up: the report book and the status rings fill.
    for _ in 0..300 {
        fail_once(&mut session);
    }
    let settled = live();
    for _ in 0..300 {
        fail_once(&mut session);
    }
    let grown = live() - settled;
    assert!(
        grown.abs() <= 16_384,
        "300 more failures grew the process heap by {grown} bytes"
    );
    assert!(session.setting_failure(setting).is_some());
}
