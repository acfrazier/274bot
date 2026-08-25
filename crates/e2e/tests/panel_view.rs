//! Live: the panel path proven headless — `host_play::run_with_io` with a
//! per-slot `FrameBuf` mailbox + `SlotInput`, no dear-app window. The
//! per-frame hook applies `set_draw(true)` (the panel's focus switch), the
//! renderer proof is a non-zero pixel snapshot, then capture goes on and a
//! click at the 3D-view center must walk the local player (operator wants
//! live proof: a missing walk is a default FAIL, not a SOFT skip).
//!
//! Run with the engine up:
//! `LIVE=1 cargo test -p e2e --test panel_view -- --ignored --test-threads=1 --nocapture`

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use common::{fail, live, options, profiles, wait_ingame};
use host::{FrameBuf, InputEv, SlotInput};
use host_play::run_with_io;

/// 3D view is 4,4–516,338 in applet coords; (256,167) is its center.
const VIEWPORT_CLICK_X: i32 = 256;
const VIEWPORT_CLICK_Y: i32 = 167;

#[test]
#[ignore = "requires a local 274 engine and LIVE=1"]
fn live_draw_area_and_capture_walk() {
    if !live() {
        return;
    }

    // The same per-slot channels the panel builds in `Session::spawn_all`:
    // one `FrameBuf` mailbox (filled while `set_draw` is on) and one
    // `SlotInput` (drained only while capture is enabled).
    let pixels = FrameBuf::new();
    let input = SlotInput::new();
    // Debug field: whether the capture Down actually reached the shell
    // (`apply_mouse_down` keeps mouse_x/y/button until a move/up). This
    // tells a blocked-walk failure apart from a dead input path.
    let mouse_applied = Arc::new(AtomicBool::new(false));
    let per_frame_mouse = Arc::clone(&mouse_applied);

    let play = run_with_io(
        &options(),
        profiles(&[("test", "test")]),
        |name| {
            if name == "test" {
                (Some(Arc::clone(&input)), Some(Arc::clone(&pixels)))
            } else {
                (None, None)
            }
        },
        // Mirrors the panel focus `set_draw` switch: the focused slot
        // renders every frame.
        move |c, _| {
            c.set_draw(true);
            if c.shell.mouse_button == 1
                && c.shell.mouse_x == VIEWPORT_CLICK_X
                && c.shell.mouse_y == VIEWPORT_CLICK_Y
            {
                per_frame_mouse.store(true, Ordering::Relaxed);
            }
        },
    );

    wait_ingame(&play, 1, Duration::from_secs(90), "panel_view");

    // Renderer proof: with draw on and scene 2, the buffer must carry a
    // non-zero frame within 10 s (an empty buffer means the set_draw/pixel
    // path never painted).
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let snap = pixels.snapshot();
        let non_zero = snap.iter().filter(|&&p| p != 0).count();
        if non_zero > 0 {
            println!(
                "PASS: panel_view renderer: non-zero draw_area frame (scene 2, draw on; {non_zero} px)"
            );
            break;
        }
        if Instant::now() >= deadline {
            fail("panel_view: draw_area stayed empty after set_draw(true) and scene 2");
        }
        thread::sleep(Duration::from_millis(250));
    }

    // Capture on (`Session::capture_on`): attach the channel and enable
    // the drain. Like the panel (which streams Move/Up from the ImGui
    // mouse), park the pointer first so the minimenu rebuilds at the click
    // point — a Down with no preceding Move fires against the stale menu
    // built at the previous pointer position.
    let (tx, rx) = mpsc::channel();
    input.connect_rx(rx);
    input.set_enabled(true);
    tx.send(InputEv::Move {
        x: VIEWPORT_CLICK_X,
        y: VIEWPORT_CLICK_Y,
    })
    .unwrap();
    thread::sleep(Duration::from_millis(100));

    let before = play.statuses()[0].clone();
    tx.send(InputEv::Down {
        button: 1,
        x: VIEWPORT_CLICK_X,
        y: VIEWPORT_CLICK_Y,
    })
    .unwrap();

    // Walk proof: the local-player tile must move. A click that does not
    // change the tile while capture and renderer are on is a default FAIL.
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let s = play.statuses()[0].clone();
        if (s.tile_x, s.tile_z) != (before.tile_x, before.tile_z) {
            println!(
                "PASS: panel_view capture walk: {} tile ({},{}) -> ({},{})",
                s.player, before.tile_x, before.tile_z, s.tile_x, s.tile_z
            );
            return;
        }
        if Instant::now() >= deadline {
            fail(&format!(
                "capture click at ({}, {}) did not change tile (was ({}, {})); mouse_applied={}",
                VIEWPORT_CLICK_X,
                VIEWPORT_CLICK_Y,
                before.tile_x,
                before.tile_z,
                mouse_applied.load(Ordering::Relaxed)
            ));
        }
        thread::sleep(Duration::from_millis(250));
    }
}
