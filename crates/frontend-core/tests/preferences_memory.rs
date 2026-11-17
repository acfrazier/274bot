//! Own test binary: the Rust memory of the preferences database path at its
//! maximum state (T5.3 S4, plan §9.2). Live bytes over every thread (the
//! writer thread allocates the parsed document and the staged bytes) and
//! their peak, against a baseline taken in the same initialized process
//! before any preference allocation.
//!
//! The fixture: a legacy document whose canonical form is exactly D bytes,
//! imported at the first open, and one core session following it. Three old
//! durable versions of it stay held through the whole run, each its own
//! parsed copy: the panel's poll can hold three overlapping old documents
//! (`crates/panel/src/session.rs:2287-2315`, then
//! `crates/frontend-core/src/session.rs:1104-1119`, then
//! `crates/frontend-core/src/session/prefs.rs:403-419`). Then three cycles:
//! the writer is held while nine saves wait (eight queued, one active) up to
//! the queue's byte bound D exactly, a tenth is refused, and the writer is
//! released. The peak tripwire covers that whole run; the retained tripwire
//! covers what stays once the old owners let go, every save settled and
//! every result was projected: the durable snapshot, the core's
//! reconciliation copy and fixed structs.
//!
//! Tripwires are the measured value plus 12.5 % headroom (`T = M + ceil(M /
//! 8)`, plan-r3.md:334), committed per target with their measurement; the
//! test never remeasures its own limit.
use std::alloc::{GlobalAlloc, Layout, System};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use api::local_state::{ErrorCode, ValidationReason};
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
/// `(M_peak, T_peak, M_retained, T_retained)`, with `T = M + H` and
/// `H = ceil(M / 8)` (plan-r3.md:334); the test checks that arithmetic.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const TRIPWIRES: Option<(isize, isize, isize, isize)> = Some(TRIPWIRES_LINUX_X86_64);
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const TRIPWIRES: Option<(isize, isize, isize, isize)> = Some(TRIPWIRES_MACOS_AARCH64);
#[cfg(not(any(
    all(target_os = "linux", target_arch = "x86_64"),
    all(target_os = "macos", target_arch = "aarch64")
)))]
const TRIPWIRES: Option<(isize, isize, isize, isize)> = None;

/// Linux x86_64. This fixture (three old owners, the full D-byte queue, three
/// cycles), measured on the S1 candidate `44d233834` in the syrah guest on
/// chardonnay, three repeats, all complete (S1-PREFS-SQLITE-1114
/// `measure/activation/peak-three-summary.json`, computed by
/// `summarize-peak-three.py`). Peak: M 10,804,395 B, H 1,350,550 B, T
/// 12,154,945 B. Retained: M 2,189,871 B, H 273,734 B, T 2,463,605 B. Each H
/// is ceil(M / 8) and each T is M + H (plan-r3.md:334).
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const TRIPWIRES_LINUX_X86_64: (isize, isize, isize, isize) =
    (10_804_395, 12_154_945, 2_189_871, 2_463_605);

/// macOS aarch64. This fixture, measured on the S1 candidate `44d233834` on
/// merlot, three repeats, all complete (S1-PREFS-SQLITE-1114
/// `measure/activation/macos-peak-three-summary.json`). Peak: M 10,804,611 B,
/// H 1,350,577 B, T 12,155,188 B. Retained: M 2,190,151 B, H 273,769 B, T
/// 2,463,920 B. Each H is ceil(M / 8) and each T is M + H (plan-r3.md:334).
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const TRIPWIRES_MACOS_AARCH64: (isize, isize, isize, isize) =
    (10_804_611, 12_155_188, 2_190_151, 2_463_920);

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

/// D, the preference document bound (`MAX_PREFERENCES_BYTES`): the fixture
/// below is calibrated to it to the byte.
const D: usize = 1 << 20;
const WAIT: Duration = Duration::from_secs(60);
/// The legacy document has this many members; each value is `VALUE_BYTES`
/// long, the first one `FIRST_EXTRA` longer.
const MEMBERS: usize = 268;
const VALUE_BYTES: usize = 3_896;
const FIRST_EXTRA: usize = 142;
/// Old durable versions held by their owners through the run, and the
/// cycles of the full queue.
const OLD_VERSIONS: usize = 3;
const CYCLES: usize = 3;
/// A save of the queue changes `SAVE_MEMBERS` members (`bulk.m000` and on),
/// each to a value of `QUEUE_VALUE_BYTES` characters, the first member of the
/// first save `QUEUE_FIRST_EXTRA` longer.
const SAVES: usize = 9;
const SAVE_MEMBERS: usize = 30;
const QUEUE_VALUE_BYTES: usize = 3_869;
const QUEUE_FIRST_EXTRA: usize = 166;

/// The legacy document: one `bulk` object of 268 members, in the pretty
/// form the product stores it in, exactly D bytes (the limit it admits).
fn maximum_document() -> Vec<u8> {
    use std::io::Write;
    assert_eq!(
        host_play::local_state::MAX_PREFERENCES_BYTES as usize,
        D,
        "the fixture is calibrated to D = 1,048,576 bytes: recalibrate it and measure again"
    );
    let mut bytes = Vec::with_capacity(D);
    bytes.extend_from_slice(b"{\n  \"bulk\": {\n");
    for member in 0..MEMBERS {
        write!(&mut bytes, "    \"m{member:03}\": \"").unwrap();
        let extra = if member == 0 { FIRST_EXTRA } else { 0 };
        bytes.resize(bytes.len() + VALUE_BYTES + extra, b'a');
        bytes.extend_from_slice(if member == MEMBERS - 1 {
            b"\"\n"
        } else {
            b"\",\n"
        });
    }
    bytes.extend_from_slice(b"  }\n}");
    assert_eq!(bytes.len(), D, "the document must fill the byte limit");
    // It is the canonical form: what the product stores is these bytes.
    let parsed: serde_json::Map<String, serde_json::Value> =
        serde_json::from_slice(&bytes).expect("a JSON object");
    assert_eq!(
        serde_json::to_vec_pretty(&parsed).expect("serializes"),
        bytes,
        "the document is not in the canonical form"
    );
    bytes
}

/// One save of the queue. The product charges a changed member at its key
/// plus its JSON-encoded value (`bulk.m000` is 9 bytes, a value of 3,872
/// characters 3,874), so the nine saves of a cycle charge exactly D bytes:
/// 270 x (9 + 3,874) + 166. `extra` adds that many bytes to the last member.
fn queue_patch(cycle: usize, round: usize, extra: usize) -> PreferencePatch {
    let mut patch = PreferencePatch::new();
    for member in 0..SAVE_MEMBERS {
        let first = if round == 0 && member == 0 {
            QUEUE_FIRST_EXTRA
        } else {
            0
        };
        let last = if member == SAVE_MEMBERS - 1 { extra } else { 0 };
        let value = format!(
            "{cycle}{round}-{}",
            "b".repeat(QUEUE_VALUE_BYTES + first + last)
        );
        patch
            .set_member(
                &["bulk", &format!("m{member:03}")],
                Some(serde_json::json!(value)),
            )
            .unwrap();
    }
    patch
}

/// Wait for a save to be durable.
fn settle(update: Update) {
    if let Update::Queued(mut update) = update {
        update
            .wait(WAIT)
            .expect("a terminal result")
            .expect("a durable save");
    }
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
    let legacy = maximum_document();
    let document_bytes = legacy.len() as isize;
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
    counter_counts_growth_and_release();
    counter_counts_realloc();
    let baseline = LIVE.load(Ordering::SeqCst);
    PEAK.store(baseline, Ordering::SeqCst);

    let service = LocalStateService::open_process(root);
    let mut session: OperatorSession<()> = OperatorSession::new(InstancePermit::SkipLock);
    session.set_spawn_workers(false);
    session.bind_local_state(Arc::clone(&service));
    let _ = session.adopt_operator_preferences();
    let preferences = service.preferences();
    let deadline = Instant::now() + WAIT;
    while preferences.snapshot().is_err() {
        assert!(Instant::now() < deadline, "the import never finished");
        std::thread::sleep(Duration::from_millis(5));
    }
    poll(&mut session);
    assert_eq!(
        preferences.snapshot().expect("durable").document()["bulk"]
            .as_object()
            .map(|bulk| bulk.len()),
        Some(MEMBERS),
        "the whole legacy document was imported"
    );

    // Three old durable versions, each held by its own owner from here to
    // the end of the run: owner `index` holds the document as it was before
    // save `index` below. Each save replaces the last member by a value of
    // the same length, so every version is D bytes and its own copy.
    let last_member = format!("m{:03}", MEMBERS - 1);
    let mut old_versions = Vec::with_capacity(OLD_VERSIONS);
    for index in 0..OLD_VERSIONS {
        old_versions.push(preferences.snapshot().expect("a durable snapshot"));
        let mut patch = PreferencePatch::new();
        patch
            .set_member(
                &["bulk", &last_member],
                Some(serde_json::json!(format!(
                    "{index}{}",
                    "a".repeat(VALUE_BYTES - 1)
                ))),
            )
            .unwrap();
        settle(preferences.begin_patch(patch).expect("admitted"));
        poll(&mut session);
    }

    for cycle in 0..CYCLES {
        let (entered_tx, entered) = mpsc::channel::<()>();
        let (release, gate) = mpsc::channel::<()>();
        let gate = Mutex::new(gate);
        let held = AtomicBool::new(false);
        service.set_write_stall_for_test(Arc::new(move || {
            if !held.swap(true, Ordering::SeqCst) {
                entered_tx.send(()).unwrap();
                let _ = gate.lock().unwrap().recv();
            }
        }));
        let mut updates = Vec::with_capacity(SAVES);
        for round in 0..SAVES {
            if round == SAVES - 1 {
                // The queue holds D bytes less this save: one byte more is
                // refused for its size, so the nine saves fill it exactly.
                let refused = preferences
                    .begin_patch(queue_patch(cycle, round, 1))
                    .expect_err("the queue admits D bytes and not one more");
                assert!(
                    matches!(
                        refused.code(),
                        Some(ErrorCode::Validation {
                            reason: ValidationReason::TooLong,
                            ..
                        })
                    ),
                    "{refused}"
                );
            }
            match preferences
                .begin_patch(queue_patch(cycle, round, 0))
                .expect("admitted")
            {
                Update::Queued(update) => updates.push(update),
                Update::Durable => panic!("a change is queued"),
            }
            if round == 0 {
                // The first save is the one active save; the other eight
                // queue behind it.
                entered
                    .recv_timeout(WAIT)
                    .expect("the writer is held in the first save");
            }
            poll(&mut session);
        }
        let tenth = preferences
            .begin_set("overflow", serde_json::json!(true))
            .expect_err("a tenth request is refused");
        assert!(
            matches!(tenth.code(), Some(ErrorCode::Busy { .. })),
            "{tenth}"
        );
        release.send(()).unwrap();
        for mut update in updates {
            update.wait(WAIT).expect("settles").expect("durable");
        }
        for _ in 0..4 {
            poll(&mut session);
        }
        assert!(
            preferences.unprojected_requests().is_empty(),
            "no retained result"
        );
        // The old versions are unchanged, and each is its own version.
        let current = preferences.snapshot().expect("durable").version();
        for (index, owner) in old_versions.iter().enumerate() {
            let value = owner.document()["bulk"][last_member.as_str()]
                .as_str()
                .expect("a string value");
            assert_eq!(value.len(), VALUE_BYTES, "old version {index}");
            let first = if index == 0 {
                b'a'
            } else {
                b'0' + (index - 1) as u8
            };
            assert_eq!(value.as_bytes()[0], first, "old version {index}");
            assert!(
                value.as_bytes()[1..].iter().all(|byte| *byte == b'a'),
                "old version {index}"
            );
            assert_ne!(owner.version(), current, "old version {index}");
            if index > 0 {
                assert_ne!(
                    owner.version(),
                    old_versions[index - 1].version(),
                    "old version {index}"
                );
            }
        }
    }
    drop(old_versions);
    let peak = PEAK.load(Ordering::SeqCst) - baseline;
    drop(preferences);
    let retained = LIVE.load(Ordering::SeqCst) - baseline;
    println!("T53_RUST_MEMORY peak={peak} retained={retained}");
    // The durable snapshot alone holds the imported document, so a retained
    // total under its size means the counter saw releases that it did not
    // see allocations for (a `realloc` it missed reads negative here).
    assert!(
        retained >= document_bytes,
        "retained {retained} < the imported document's {document_bytes} bytes: the counter missed allocations"
    );
    // At its peak the fixture holds three old versions of the document, each
    // its own parsed copy of 268 values, and the values of the nine saves
    // that wait. A counter that saw the work reads at least that much; a
    // peak under the floor means the counter stopped counting.
    let document_values = MEMBERS * VALUE_BYTES + FIRST_EXTRA;
    let queued_values = SAVES * SAVE_MEMBERS * QUEUE_VALUE_BYTES + QUEUE_FIRST_EXTRA;
    let floor = (OLD_VERSIONS * document_values + queued_values) as isize;
    assert!(
        peak >= floor,
        "peak {peak} < the {floor} bytes of three old versions and the full queue: the counting allocator saw none of the work"
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
/// [`counter_counts_realloc`] checks the third way memory changes hands.
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

/// The counter moves with every `realloc`, in both directions, by the
/// difference of the two sizes. Every `Vec` and `String` the product grows
/// or shrinks goes through `realloc`, so a counter that missed it would
/// read a fraction of the true peak and a negative live total once the
/// buffers were released, and both tripwires above would pass.
fn counter_counts_realloc() {
    const SMALL: usize = 1 << 20;
    const LARGE: usize = 3 << 20;
    // Other threads of the test harness allocate and free a few hundred
    // bytes; a missed or doubled `realloc` is off by a megabyte or more.
    const SLACK: isize = 64 << 10;
    let before = LIVE.load(Ordering::SeqCst);
    PEAK.store(before, Ordering::SeqCst);
    let mut block = std::hint::black_box(Vec::<u8>::with_capacity(SMALL));
    block.resize(SMALL, 1);
    // Growth: the vector asks the allocator to resize its one block.
    block.reserve_exact(LARGE - SMALL);
    std::hint::black_box(&mut block);
    let grown_to = block.capacity() as isize;
    assert!(grown_to > SMALL as isize, "the vector grew its block");
    let grown = LIVE.load(Ordering::SeqCst) - before;
    assert!(
        (grown_to - SLACK..grown_to + SLACK).contains(&grown),
        "the counter holds {grown} bytes for one block grown by realloc to {grown_to} bytes"
    );
    assert!(
        PEAK.load(Ordering::SeqCst) - before >= grown_to - SLACK,
        "the peak did not cover a block grown by realloc to {grown_to} bytes"
    );
    // Shrinking: the same block gives back all but `SMALL` bytes.
    block.shrink_to(SMALL);
    std::hint::black_box(&mut block);
    let shrunk_to = block.capacity() as isize;
    assert!(shrunk_to < grown_to, "the vector shrank its block");
    let shrunk = LIVE.load(Ordering::SeqCst) - before;
    assert!(
        (shrunk_to - SLACK..shrunk_to + SLACK).contains(&shrunk),
        "the counter holds {shrunk} bytes for one block shrunk by realloc to {shrunk_to} bytes"
    );
    drop(block);
    let after = LIVE.load(Ordering::SeqCst) - before;
    assert!(
        (-SLACK..SLACK).contains(&after),
        "the counter holds {after} bytes after the grown and shrunk block was released"
    );
}
