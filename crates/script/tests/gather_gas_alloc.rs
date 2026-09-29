//! Own test binary (it installs a counting global allocator). `GAS_ROCK_IDS` is a plain `Set` built at module
//! evaluation: a thousand `has` calls cost no Rust allocation, so nothing rebuilds the ids or crosses into Rust per
//! call. The controls and the measured tick differ only in how many `has` calls the tick makes.
use client::io::ClientRevision;
use script::{LoadIsolate, LoadShape};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Counting;

/// Allocation and reallocation events on every thread: the isolate thread runs the script, and the test thread
/// is parked on the probe barrier while it does.
static EVENTS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        EVENTS.fetch_add(1, Ordering::Relaxed);
        System.alloc(layout)
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        EVENTS.fetch_add(1, Ordering::Relaxed);
        System.alloc_zeroed(layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        EVENTS.fetch_add(1, Ordering::Relaxed);
        System.realloc(ptr, layout, new_size)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

const SRC: &str = r#"
import { GAS_ROCK_IDS } from '../../data/miningRocks.js';
export default class T extends LoopingBot {
    loop() {
        let hits = 0;
        for (let i = 0; i < globalThis.__n; i++) {
            if (GAS_ROCK_IDS.has(2119 + (i % 30))) hits++;
        }
        globalThis.__probe = hits;
    }
}
"#;

/// Events one tick costs end to end: dispatch, script and the probe barrier that proves it finished.
fn tick_events(iso: &LoadIsolate, tick: u64, checks: u32) -> (usize, serde_json::Value) {
    iso.probe(&format!("globalThis.__n = {checks}")).unwrap();
    let before = EVENTS.load(Ordering::Relaxed);
    iso.on_game_tick(tick);
    let hits = iso.probe("globalThis.__probe").unwrap();
    (EVENTS.load(Ordering::Relaxed) - before, hits)
}

#[test]
fn repeated_membership_checks_allocate_nothing_after_the_first_touch() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let iso = LoadIsolate::spawn_with_game_data(SRC.into(), LoadShape::CompatClass, vec![], data)
        .unwrap();
    assert_eq!(iso.probe("1").unwrap(), serde_json::json!(1));

    // A first tick, then warm ticks so lazy runtime state is settled.
    let (_, hits) = tick_events(&iso, 1, 1);
    assert_eq!(hits, serde_json::json!(1));
    let mut tick = 2;
    for _ in 0..5 {
        tick_events(&iso, tick, 1000);
        tick += 1;
    }

    // Per 30 consecutive ids, 21 are gas rocks (2119..=2139): 33 full cycles plus ids 2119..=2128.
    let expected_hits = 33 * 21 + 10;
    let mut control = usize::MAX;
    let mut measured = usize::MAX;
    for _ in 0..5 {
        control = control.min(tick_events(&iso, tick, 0).0);
        tick += 1;
        let (events, hits) = tick_events(&iso, tick, 1000);
        tick += 1;
        assert_eq!(hits, serde_json::json!(expected_hits));
        measured = measured.min(events);
    }
    iso.join();
    eprintln!("allocation events per tick: 0 checks = {control}, 1000 checks = {measured}");
    // A per-call rebuild costs several Rust allocation events per `has`; allow only tick-to-tick noise.
    assert!(
        measured <= control + 32,
        "1000 has calls added {} allocation events over the empty tick ({control})",
        measured - control.min(measured)
    );
}
