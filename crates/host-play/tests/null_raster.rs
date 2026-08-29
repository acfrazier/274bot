//! Live: two slots, only `test` set_draw; `test2` stays draw-off.
//!
//! The draw-off guard is asserted through the per-slot `FrameBuf`
//! generation (the old `game_draw_enters`/`title_screen_draw_enters`
//! counters are gone from `SlotStatus`, M2). The mechanism:
//! `raster_this_tick` gates on `client.draw`, so a draw-off slot never
//! dispatches the renderer, never produces a `FrameOutput`, and never
//! reaches `FrameBuf::store` — its generation stays 0. The draw-on slot
//! rasters at the 1 fps watch cadence (no capture, no full-rate in this
//! harness), so its generation grows. `per_frame` applies the focus
//! `set_draw` switch exactly as the panel does; the per-frame hook
//! counter keeps the wiring liveness check that used to ride on the draw
//! counters.
//!
//! LIVE=1 cargo test -p host-play --test null_raster -- --ignored --test-threads=1 --nocapture

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use common::{fail, live, options, profiles, wait_ingame};
use host::FrameBuf;
use host_play::run_with_io;

#[test]
#[ignore = "requires a local 274 engine and LIVE=1"]
fn live_draw_off_never_paints() {
    if !live() {
        return;
    }
    // One `FrameBuf` per slot; a stored frame bumps its generation.
    let test_pixels = FrameBuf::new();
    let test2_pixels = FrameBuf::new();
    // Per-slot hook frame counter (per_frame wiring liveness).
    let frames = Arc::new(Mutex::new(HashMap::new()));
    let play = run_with_io(
        &options(),
        profiles(&[("test", "test"), ("test2", "test2")]),
        {
            let test_pixels = Arc::clone(&test_pixels);
            let test2_pixels = Arc::clone(&test2_pixels);
            move |name| {
                if name == "test" {
                    (None, Some(Arc::clone(&test_pixels)))
                } else {
                    (None, Some(Arc::clone(&test2_pixels)))
                }
            }
        },
        {
            let frames = Arc::clone(&frames);
            move |c, name| {
                *frames
                    .lock()
                    .unwrap()
                    .entry(name.to_string())
                    .or_insert(0u64) += 1;
                c.set_draw(name == "test");
            }
        },
    );
    wait_ingame(&play, 2, Duration::from_secs(120), "null_raster");

    let before: HashMap<String, u64> = frames.lock().unwrap().clone();
    let g0 = test_pixels.generation();
    // Draw-on rasters at 1 fps (no capture/full-rate), so 3 s is a few
    // paints; draw-off must stay at exactly 0 the whole time.
    thread::sleep(Duration::from_secs(3));
    let after = frames.lock().unwrap().clone();
    let g1 = test_pixels.generation();
    let g2 = test2_pixels.generation();

    for name in ["test", "test2"] {
        let b = before.get(name).copied().unwrap_or(0);
        let a = after.get(name).copied().unwrap_or(0);
        if a <= b {
            fail(&format!("null_raster: {name} per-frame hook did not run"));
        }
    }
    if g1 <= g0 {
        fail(&format!(
            "null_raster: draw-on test never painted (gen {g0} -> {g1})"
        ));
    }
    if g2 != 0 {
        fail(&format!("null_raster: draw-off test2 painted (gen {g2})"));
    }
    println!(
        "null_raster: test gen {g0} -> {g1} (draw on) test2 gen {g2} (draw off) \
         frames test={} test2={}",
        after.get("test").copied().unwrap_or(0),
        after.get("test2").copied().unwrap_or(0)
    );
    println!("PASS: null_raster draw-on paints, draw-off never");
}
