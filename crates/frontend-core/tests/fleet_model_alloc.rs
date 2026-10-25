//! Own test binary: the fleet model's cost. A steady refresh (nothing
//! moved) allocates nothing, a rebuild of an unchanged universe reuses its
//! buffers, and the model's live heap grows with the profile count only
//! (a few hundred bytes per profile row), never per frame.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::path::PathBuf;

use frontend_core::command::{DrawSwitch, OperatorCommand, RasterMode};
use frontend_core::surface::{SlotAttach, SlotSurface, SpawnProfile};
use frontend_core::{FleetModel, Models, OperatorSession, Targets};
use host_play::{InstancePermit, PlayOptions};
use vault::{Profile, ProfileSettings, Vault};

struct Counting;

thread_local! {
    static EVENTS: Cell<usize> = const { Cell::new(0) };
    static LIVE: Cell<isize> = const { Cell::new(0) };
}

fn note(delta: isize, event: bool) {
    let _ = LIVE.try_with(|live| live.set(live.get() + delta));
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
    LIVE.with(Cell::get)
}

/// A surface with no devices and no models (Load needs neither).
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
    let dir =
        std::env::temp_dir().join(format!("274bot-fleet-alloc-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("test directory");
    dir
}

/// A session over a vault of `loaded + unloaded` profiles with the first
/// `loaded` of them loaded (no worker threads) and projected.
fn session(label: &str, loaded: usize, unloaded: usize) -> OperatorSession<()> {
    let dir = scratch(label);
    // Hermetic: no world pack from the operator's home (see memory_notice.rs).
    std::env::set_var("NAV_PACK", dir.join("empty.navpack"));
    let play = host_play::run_with_io(
        &PlayOptions {
            host: "127.0.0.1".into(),
            transport: host_play::Transport::Tcp,
            port: 43594,
            cache_dir: dir.join("cache").to_string_lossy().into_owned(),
            lowmem: true,
            mainland: false,
        },
        vec![],
        |_| (None, None),
        |_, _, _| {},
    );
    let mut vault = Vault::create(&dir.join("vault"), "test-passphrase-01").expect("vault");
    for index in 0..loaded + unloaded {
        vault
            .upsert(Profile {
                username: format!("bot{index:04}"),
                password: "pw".into(),
                uid: 1_000 + index as i32,
                settings: ProfileSettings {
                    auto_login: false,
                    ..ProfileSettings::default()
                },
            })
            .expect("profile");
    }
    let mut session: OperatorSession<()> = OperatorSession::new(InstancePermit::SkipLock);
    session.set_spawn_workers(false);
    session.start_for_test(vault, play);
    let names: Vec<String> = (0..loaded).map(|index| format!("bot{index:04}")).collect();
    if !names.is_empty() {
        session
            .submit(OperatorCommand::Load, Targets::Profiles(&names), &mut Bare)
            .expect("load");
    }
    session.poll();
    session
}

#[test]
fn a_steady_refresh_allocates_nothing() {
    for (loaded, unloaded) in [(1, 0), (10, 10), (100, 100)] {
        let mut session = session("steady", loaded, unloaded);
        let mut model = FleetModel::new();
        assert!(model.refresh(&session));
        assert_eq!(model.rows().len(), loaded + unloaded);
        let mut allocated = 0;
        for _ in 0..1_000 {
            session.poll();
            let startup_before = events();
            assert!(session.prepare_startup());
            assert_eq!(events() - startup_before, 0, "ready startup gate allocated");
            let before = events();
            assert!(!model.refresh(&session), "nothing moved");
            allocated += events() - before;
        }
        assert_eq!(
            allocated, 0,
            "{loaded} loaded, {unloaded} not: steady refresh allocated"
        );
    }
}

#[test]
fn a_rebuild_of_an_unchanged_universe_reuses_its_buffers() {
    let mut session = session("rebuild", 50, 50);
    let mut model = FleetModel::new();
    model.refresh(&session);
    // Bump the profiles generation without changing a row (the fixture
    // vault access settles writes first, outside the measured call).
    let _ = session.vault_mut();
    let rows = model.rows_generation();
    let before = events();
    assert!(model.refresh(&session), "the vault moved, so it rebuilds");
    assert_eq!(events() - before, 0, "the rebuild reused every buffer");
    assert_eq!(model.rows_generation(), rows + 1);
}

/// The model's live heap by profile count: what a front end holds once,
/// process-wide (never per bot). Printed for the slice report and bounded
/// so an accidental per-row copy fails.
#[test]
fn the_model_heap_grows_with_the_profile_count_only() {
    // Measured 2026-10-20: 792 bytes for one row (vector growth from
    // empty), then 255–330 bytes per row (330 at 600 rows is vector
    // capacity slack). The ceilings keep headroom and fail on any second
    // per-row copy.
    const FIXED_CEILING: isize = 1_024;
    const PER_ROW_CEILING: isize = 400;
    let mut lines = Vec::new();
    for (loaded, unloaded) in [(1, 0), (10, 0), (50, 0), (100, 0), (100, 100), (100, 500)] {
        let session = session("heap", loaded, unloaded);
        let mut model = FleetModel::new();
        let before = live();
        model.refresh(&session);
        let bytes = live() - before;
        let rows = (loaded + unloaded) as isize;
        lines.push(format!(
            "loaded={loaded} unloaded={unloaded} rows={rows} model_heap_bytes={bytes} per_row={}",
            bytes / rows
        ));
        assert!(
            bytes <= FIXED_CEILING + PER_ROW_CEILING * rows,
            "{bytes} bytes for {rows} rows exceeds {FIXED_CEILING} + {PER_ROW_CEILING} per row"
        );
        drop(model);
        drop(session);
    }
    eprintln!("FLEET-MODEL-HEAP\n{}", lines.join("\n"));
}
