//! Own test binary: the whole frontend-core frame. One refresh pass is
//! what every front end runs per frame before it draws: the session poll
//! (`OperatorSession::poll_at` — removals, host pump, transitions,
//! settlements, the projections, the once-a-second capacity and resource
//! cadences) plus the fleet model refresh the front ends drive on top.
//! Nothing moved means nothing allocated: a warm whole frame that grows
//! would be a per-frame leak in the core every front end inherits.
//!
//! The per-profile heap stays separately bounded by `fleet_model_alloc.rs`
//! (1,024 B fixed + 400 B per row); this gate bounds the per-frame cost.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::path::PathBuf;
use std::time::{Duration, Instant};

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
    let dir = std::env::temp_dir().join(format!(
        "274bot-whole-frame-alloc-{label}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("test directory");
    dir
}

/// A session over a vault of `loaded + unloaded` profiles with the first
/// `loaded` of them loaded (no worker threads) and projected.
fn session(label: &str, loaded: usize, unloaded: usize) -> OperatorSession<()> {
    let dir = scratch(label);
    // Hermetic: no world pack from the operator's home.
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

/// One whole frame: the session's poll pass plus the fleet model refresh
/// the front ends drive on top of it. `now` is the frame's clock, so the
/// measured frames repeat one instant exactly (nothing can move).
fn whole_frame(session: &mut OperatorSession<()>, model: &mut FleetModel, now: Instant) -> bool {
    session.poll_at(now);
    model.refresh(session)
}

#[test]
fn the_whole_frame_refresh_allocates_nothing() {
    for (loaded, unloaded) in [(1, 0), (10, 10), (100, 100)] {
        let mut session = session("steady", loaded, unloaded);
        let mut model = FleetModel::new();
        assert!(model.refresh(&session));
        assert_eq!(model.rows().len(), loaded + unloaded);

        // Warm with advancing virtual time so every once-a-second cadence
        // (the capacity gate, the resource sampler) fires and settles.
        let t0 = Instant::now();
        for tick in 0..4u32 {
            assert!(
                !whole_frame(
                    &mut session,
                    &mut model,
                    t0 + Duration::from_secs(tick as u64)
                ),
                "warm frame {tick} moved"
            );
        }
        // One more due sample arms the resource meter's probe thread once
        // more; the probe completes on the real clock. Drain it with
        // real-time polls so no publication can land in the measured
        // window (at the fixed steady instant the cadences are inside
        // their periods and never request again).
        session.poll_at(t0 + Duration::from_secs(6));
        let drain_deadline = Instant::now() + Duration::from_secs(5);
        loop {
            std::thread::sleep(Duration::from_millis(20));
            let before = events();
            session.poll_at(t0 + Duration::from_millis(6_500));
            if events() > before {
                break;
            }
            assert!(
                Instant::now() < drain_deadline,
                "the resource meter never published during the drain"
            );
        }
        assert!(!model.refresh(&session), "the drain frame moved");

        // The measured frames repeat one instant: both cadences skip, the
        // rows and statuses are unchanged, and no probe is in flight.
        let steady = t0 + Duration::from_millis(6_500);
        assert!(
            !whole_frame(&mut session, &mut model, steady),
            "the steady frame moved"
        );
        let before_events = events();
        let before_live = live();
        for _ in 0..100 {
            assert!(
                !whole_frame(&mut session, &mut model, steady),
                "nothing moved"
            );
        }
        assert_eq!(
            events() - before_events,
            0,
            "{loaded} loaded, {unloaded} not: a steady whole-frame refresh allocated"
        );
        assert_eq!(
            live() - before_live,
            0,
            "{loaded} loaded, {unloaded} not: a steady whole-frame refresh retained"
        );
        drop(model);
        drop(session);
    }
}
