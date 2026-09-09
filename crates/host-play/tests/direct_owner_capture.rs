//! Generated borrowed COW regression; no Play, account, server or cache.

// Isolated TEST executable only; production capture retains System unchanged.
#[cfg(feature = "memory-owner-capture")]
#[global_allocator]
static ALLOC: host_play::memory::CountingAllocator = host_play::memory::CountingAllocator;

#[cfg(all(feature = "memory-owner-capture", unix))]
fn thread_cpu_ns() -> u64 {
    let mut t: libc::timespec = unsafe { std::mem::zeroed() };
    assert_eq!(
        unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) },
        0
    );
    (t.tv_sec as u64) * 1_000_000_000 + t.tv_nsec as u64
}

#[cfg(feature = "memory-owner-capture")]
#[test]
fn real_pre_observe_and_nav_seams_capture_independent_shells() {
    use api::snapshot::GameSnapshot;
    use client::client::{Client, ClientConfig};
    use host::owner_capture as h;
    let client = Client::new(ClientConfig {
        host: "127.0.0.1".into(),
        port: 1,
        cache_dir: "/nonexistent-owner-fixture".into(),
        members: false,
        lowmem: true,
    });
    let host = GameSnapshot::new();
    let nav = GameSnapshot::new();
    h::init_enabled(true);
    let token = host_play::owner_capture::register_slot();
    let mut scratch = h::CowScratch::new();
    let counts = host_play::memory::rust_allocator_counts;
    let positive_before = counts();
    let positive = std::hint::black_box(Vec::<u8>::with_capacity(67));
    assert!(counts().0 > positive_before.0);
    drop(positive);
    host_play::owner_capture::request_phase(0, token).unwrap();
    let before = counts();
    #[cfg(unix)]
    let cpu = thread_cpu_ns();
    let wall = std::time::Instant::now();
    h::pre_observe_hook(&client, &host);
    host_play::owner_capture::observe_entry_nav(
        &nav,
        token,
        &client.ifaces_mut,
        &client.ifaces_mut,
        &mut scratch,
    );
    let elapsed = wall.elapsed().as_nanos();
    #[cfg(unix)]
    let cpu_ns = thread_cpu_ns() - cpu;
    let after = counts();
    assert_eq!(after.0 - before.0, 0, "borrowed census allocated");
    assert_eq!(
        after.1 - before.1,
        0,
        "borrowed census requested heap bytes"
    );
    #[cfg(unix)]
    println!("GENERATED_OBSERVER {{\"allocations\":{},\"allocated_bytes\":{},\"thread_cpu_ns\":{},\"wall_ns\":{},\"scratch_reserved_bytes\":{},\"native_qualified\":false}}", after.0-before.0, after.1-before.1, cpu_ns, elapsed, scratch.reserved_bytes());
    let fragments = h::global_mailbox().drain_fragments();
    assert_eq!(fragments.len(), 1);
    let f = &fragments[0];
    assert!(
        f.complete,
        "{:?}; rows={}, visits={}",
        f.reason,
        f.rows.len(),
        f.visits
    );
    assert_eq!(f.epochs[1].unwrap().source, "host_snapshot");
    assert_eq!(f.epochs[2].unwrap().source, "nav_snapshot");
    assert!(f.rows.iter().any(|r| r.owner == "host_snapshot"));
    assert!(f.rows.iter().any(|r| r.owner == "nav_snapshot"));
    assert!(h::take_staging_fragment().is_none());
    // Enabled without a request, then runtime OFF: neither starts a census.
    let frame = h::current_frame();
    h::pre_observe_hook(&client, &host);
    assert!(h::take_staging_fragment().is_none());
    assert_eq!(h::current_frame(), frame.wrapping_add(1));
    h::init_enabled(false);
    h::pre_observe_hook(&client, &host);
    assert!(h::take_staging_fragment().is_none());
    assert_eq!(h::current_frame(), frame.wrapping_add(1));
}

#[cfg(feature = "memory-owner-capture")]
#[test]
fn cross_index_template_alias_is_shared_without_retaining_arcs() {
    use api::owner_capture::Budget;
    use client::config::IfTypeMut;
    use host::owner_capture::CowScratch;
    use host_play::owner_capture::account_ifaces_cow;
    use std::sync::Arc;
    let shared = Arc::new(IfTypeMut {
        text: String::with_capacity(117),
        ..Default::default()
    });
    let private = Arc::new(IfTypeMut {
        text: String::with_capacity(89),
        ..Default::default()
    });
    let template = Arc::new(vec![Some(shared.clone()), None]);
    let overlay = Arc::new(vec![
        Some(private.clone()),
        Some(shared.clone()),
        Some(private.clone()),
    ]);
    let before = (
        Arc::strong_count(&shared),
        Arc::strong_count(&private),
        Arc::strong_count(&template),
        Arc::strong_count(&overlay),
    );
    let result = account_ifaces_cow(
        &template,
        &overlay,
        &mut CowScratch::new(),
        &mut Budget::new(),
    );
    assert!(result.complete, "{:?}", result.reason);
    let private_row = result
        .rows
        .iter()
        .find(|r| r.field == "private_entries")
        .unwrap();
    assert_eq!(private_row.occupied_count, Some(1));
    assert!(
        result
            .rows
            .iter()
            .any(|r| r.field == "template_outer_shared"),
        "divergence must not hide the still-live template outer payload"
    );
    assert_eq!(
        private_row.nested_capacity_bytes,
        Some(private.text.capacity() as u64)
    );
    assert_eq!(
        before,
        (
            Arc::strong_count(&shared),
            Arc::strong_count(&private),
            Arc::strong_count(&template),
            Arc::strong_count(&overlay)
        )
    );
}
