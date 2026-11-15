//! Own test binary: the Rust memory of the preferences database path at its
//! maximum state (T5.3 S4, plan §9.2). Live bytes over every thread (the
//! writer thread allocates the parsed document and the staged bytes) and
//! their peak, against a baseline taken in the same initialized process
//! before any preference allocation.
//!
//! The fixture: a D-sized legacy document imported at the first open, one
//! core session following it, then the writer held while nine saves wait
//! (eight queued, one active) up to the queue's byte bound, then released.
//! The peak tripwire covers that whole run; the retained tripwire covers
//! what stays once every save settled and every result was projected: the
//! durable snapshot, the core's reconciliation copy and fixed structs.
//!
//! Tripwires are the measured value plus 12.5 % headroom (`T = M + ceil(M /
//! 8)`), committed per target with their measurement (evidence
//! `T5.3-S4/resource-chardonnay-1/`); the test never remeasures its own limit.
use std::alloc::{GlobalAlloc, Layout, System};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use frontend_core::command::DrawSwitch;
use frontend_core::surface::{SlotAttach, SlotSurface, SpawnProfile};
use frontend_core::{Models, OperatorSession};
use host_play::local_state::{LocalStateService, PreferencePatch, StateRoot, Update};
use host_play::InstancePermit;

struct Counting;

/// Live bytes over every thread, and their high-water mark.
static LIVE: AtomicIsize = AtomicIsize::new(0);
static PEAK: AtomicIsize = AtomicIsize::new(0);

fn note(delta: isize) {
    let now = LIVE.fetch_add(delta, Ordering::Relaxed) + delta;
    PEAK.fetch_max(now, Ordering::Relaxed);
}

// SAFETY: every method forwards its arguments unchanged to `System`, which
// upholds the `GlobalAlloc` contract; the counters are plain atomics.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note(layout.size() as isize);
        // SAFETY: same layout the caller passed.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note(layout.size() as isize);
        // SAFETY: same layout the caller passed.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note(new_size as isize - layout.size() as isize);
        // SAFETY: `ptr` and `layout` come from a prior allocation of ours.
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note(-(layout.size() as isize));
        // SAFETY: `ptr` and `layout` come from a prior allocation of ours.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// The measured tripwires of this target (bytes over the baseline):
/// `(M_peak, T_peak, M_retained, T_retained)`, with `T = M + ceil(M / 8)`.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const TRIPWIRES: Option<(isize, isize, isize, isize)> = Some(TRIPWIRES_LINUX_X86_64);
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const TRIPWIRES: Option<(isize, isize, isize, isize)> = Some(TRIPWIRES_MACOS_AARCH64);
#[cfg(not(any(
    all(target_os = "linux", target_arch = "x86_64"),
    all(target_os = "macos", target_arch = "aarch64")
)))]
const TRIPWIRES: Option<(isize, isize, isize, isize)> = None;

/// Linux x86_64, measured on chardonnay 2026-10-28 (T5.3-S4
/// `resource-chardonnay-1/rust-memory.log`): peak 5,920,519 B, retained
/// 1,993,919 B; T = M + ceil(M / 8).
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const TRIPWIRES_LINUX_X86_64: (isize, isize, isize, isize) =
    (5_920_519, 6_660_584, 1_993_919, 2_243_159);

/// macOS aarch64, measured on merlot 2026-11-14 (S1-PREFS-SQLITE-1114
/// `measure/rust-memory-macos.txt`, six runs, every one peak 5,920,907 B
/// and retained 1,994,323 B); T = M + ceil(M / 8).
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const TRIPWIRES_MACOS_AARCH64: (isize, isize, isize, isize) =
    (5_920_907, 6_661_021, 1_994_323, 2_243_614);

/// A surface with no devices and no models.
struct Bare;

impl SlotSurface for Bare {
    type Io = ();

    fn attach(
        &mut self,
        _name: &str,
        _profile: &mut SpawnProfile<'_>,
        _retained: Option<()>,
    ) -> SlotAttach<()> {
        SlotAttach::new((), None, None)
    }

    fn lifetime_reset(&mut self, _name: &str, _walk_was_armed: bool) {}

    fn released(&mut self, _name: &str) {}

    fn models(&mut self) -> Models<'_> {
        Models::none()
    }

    fn draw_changed(&mut self, _switch: DrawSwitch, _on: bool) {}
}

fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("274bot-pref-memory-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("test directory");
    dir
}

/// The legacy document: 250 members of about 3.9 KiB under one object,
/// just under D = 1 MiB as the product stores it.
fn d_sized() -> Vec<u8> {
    let members = (0..250)
        .map(|n| format!("\"m{n:03}\":\"{}\"", "a".repeat(3_900)))
        .collect::<Vec<_>>()
        .join(",");
    format!("{{\"bulk\":{{{members}}}}}").into_bytes()
}

fn poll(session: &mut OperatorSession<()>) {
    session.poll_preferences(&mut Bare);
}

#[test]
#[cfg_attr(
    not(any(
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64")
    )),
    ignore = "no measured tripwire for this target yet (plan 9.2: measure it before the target ships)"
)]
fn preferences_peak_and_retained_rust_memory_stay_within_their_measured_tripwires() {
    let dir = scratch();
    let root = StateRoot::new(dir.join(".274bot"));
    std::fs::create_dir_all(root.path()).expect("root");
    let legacy = d_sized();
    std::fs::write(root.preferences_file(), &legacy).expect("legacy file");
    drop(legacy);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::set_permissions(
            root.preferences_file(),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
    }
    let (release, gate) = mpsc::channel::<()>();
    let gate = Mutex::new(gate);
    let held = Arc::new(AtomicBool::new(false));
    counter_counts_growth_and_release();
    let baseline = LIVE.load(Ordering::SeqCst);
    PEAK.store(baseline, Ordering::SeqCst);

    let service = LocalStateService::open_process(root);
    let mut session: OperatorSession<()> = OperatorSession::new(InstancePermit::SkipLock);
    session.set_spawn_workers(false);
    session.bind_local_state(Arc::clone(&service));
    let _ = session.adopt_operator_preferences();
    let preferences = service.preferences();
    let deadline = Instant::now() + Duration::from_secs(60);
    while preferences.snapshot().is_err() {
        assert!(Instant::now() < deadline, "the import never finished");
        std::thread::sleep(Duration::from_millis(5));
    }
    poll(&mut session);

    let holding = Arc::clone(&held);
    service.set_write_stall_for_test(Arc::new(move || {
        if !holding.swap(true, Ordering::SeqCst) {
            let _ = gate.lock().unwrap().recv();
        }
    }));
    let mut updates = Vec::new();
    for round in 0..9 {
        let mut patch = PreferencePatch::new();
        for member in 0..25 {
            patch
                .set_member(
                    &["bulk", &format!("m{:03}", round * 25 + member)],
                    Some(serde_json::json!(format!("{round}-{}", "b".repeat(3_800)))),
                )
                .unwrap();
        }
        match preferences.begin_patch(patch).expect("admitted") {
            Update::Queued(update) => updates.push(update),
            Update::Durable => panic!("a change is queued"),
        }
        poll(&mut session);
    }
    release.send(()).unwrap();
    for mut update in updates {
        update
            .wait(Duration::from_secs(60))
            .expect("settles")
            .expect("durable");
    }
    for _ in 0..4 {
        poll(&mut session);
    }
    assert!(
        preferences.unprojected_requests().is_empty(),
        "no retained result"
    );
    let peak = PEAK.load(Ordering::SeqCst) - baseline;
    drop(preferences);
    let retained = LIVE.load(Ordering::SeqCst) - baseline;
    println!("T53_RUST_MEMORY peak={peak} retained={retained}");
    // The queued patches alone hold 9 x 25 values of 3,800 bytes while the
    // writer is stalled, so a counter that saw the work reads at least that
    // much. A peak under the floor means the counter stopped counting.
    let floor = 9 * 25 * 3_800;
    assert!(
        peak >= floor,
        "peak {peak} < the queued patches' {floor} bytes: the counting allocator saw none of the work"
    );
    let Some((m_peak, t_peak, m_retained, t_retained)) = TRIPWIRES else {
        return;
    };
    assert_eq!(
        t_peak,
        m_peak + (m_peak + 7) / 8,
        "the peak headroom is 12.5 %"
    );
    assert_eq!(
        t_retained,
        m_retained + (m_retained + 7) / 8,
        "the retained headroom is 12.5 %"
    );
    assert!(
        peak <= t_peak,
        "peak {peak} > tripwire {t_peak} (measured {m_peak})"
    );
    assert!(
        retained <= t_retained,
        "retained {retained} > tripwire {t_retained} (measured {m_retained})"
    );
    drop(session);
    drop(service);
    let _ = std::fs::remove_dir_all(dir);
}

/// The counter moves with allocation and back with release: a counter
/// that ignores its deltas would let every tripwire above pass at zero.
/// This target runs one test, so no other thread allocates meanwhile.
fn counter_counts_growth_and_release() {
    const SIZE: usize = 1 << 20;
    let before = LIVE.load(Ordering::SeqCst);
    PEAK.store(before, Ordering::SeqCst);
    let block = std::hint::black_box(vec![1_u8; SIZE]);
    let grown = LIVE.load(Ordering::SeqCst) - before;
    assert!(
        grown >= SIZE as isize,
        "the counter grew {grown} bytes for a {SIZE}-byte allocation"
    );
    assert!(
        PEAK.load(Ordering::SeqCst) - before >= SIZE as isize,
        "the peak did not cover a live {SIZE}-byte allocation"
    );
    drop(block);
    let after = LIVE.load(Ordering::SeqCst) - before;
    assert!(
        after < SIZE as isize,
        "the counter kept {after} bytes after the {SIZE}-byte block was released"
    );
}
