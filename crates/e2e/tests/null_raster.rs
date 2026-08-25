//! Live: two slots, only `test` set_draw; `test2` stays draw-off.
//!
//! The old `game_draw_enters`/`title_screen_draw_enters` counters are gone
//! from `SlotStatus` (M2), so the surviving regression is per-frame
//! wiring: the per-frame hook must run on both slots' threads (this was
//! what incremented the draw counters). The set_draw split stays so the
//! per_frame → `set_draw` path is exercised exactly as the panel applies
//! it from focus.
//!
//! LIVE=1 cargo test -p e2e --test null_raster -- --ignored --test-threads=1 --nocapture

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use common::{fail, live, options, profiles, wait_ingame};
use host_play::run_with_io;

#[test]
#[ignore = "requires a local 274 engine and LIVE=1"]
fn live_per_frame_hook_runs_on_every_slot() {
    if !live() {
        return;
    }
    // Per-slot hook frame counter (the draw counters this test used to
    // read are gone; the hook invocation is the surviving observable).
    let frames = Arc::new(Mutex::new(HashMap::new()));
    let play = run_with_io(
        &options(),
        profiles(&[("test", "test"), ("test2", "test2")]),
        |_| (None, None),
        {
            let frames = Arc::clone(&frames);
            move |c, name| {
                *frames.lock().unwrap().entry(name.to_string()).or_insert(0u64) += 1;
                c.set_draw(name == "test");
            }
        },
    );
    wait_ingame(&play, 2, Duration::from_secs(120), "null_raster");
    let before: HashMap<String, u64> = frames.lock().unwrap().clone();
    thread::sleep(Duration::from_secs(3));
    let after = frames.lock().unwrap().clone();
    for name in ["test", "test2"] {
        let b = before.get(name).copied().unwrap_or(0);
        let a = after.get(name).copied().unwrap_or(0);
        if a <= b {
            fail(&format!("null_raster: {name} per-frame hook did not run"));
        }
    }
    let (t, t2) = (
        after.get("test").copied().unwrap_or(0),
        after.get("test2").copied().unwrap_or(0),
    );
    println!("null_raster: per-frame hook frames test={t} test2={t2}");
    println!("PASS: null_raster per-frame hook ran on both slots");
}
