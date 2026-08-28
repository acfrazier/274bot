//! Live capture: log in test/test near the bank, rotate the camera, and
//! save a 360-degree sweep of the 3D viewport as PPMs for inspection.
//!
//! `LIVE=1 cargo test -p e2e --test booth_capture -- --ignored --test-threads=1 --nocapture`

mod common;

use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use client::render::backend::FrameOutput;
use common::{fail, live, options, profiles, wait_ingame};
use host::{FrameBuf, InputEv, SlotInput};
use host_play::run_with_io;

const W: usize = 765;
const H: usize = 503;
const VX: usize = 4;
const VY: usize = 4;
const VW: usize = 512;
const VH: usize = 334;

fn save_ppm(frame: &[i32], path: &str) {
    let mut ppm = format!("P6\n{VW} {VH}\n255\n").into_bytes();
    for y in VY..VY + VH {
        for x in VX..VX + VW {
            let p = frame[y * W + x];
            ppm.push(((p >> 16) & 0xff) as u8);
            ppm.push(((p >> 8) & 0xff) as u8);
            ppm.push((p & 0xff) as u8);
        }
    }
    std::fs::write(path, ppm).unwrap();
    println!("PASS: wrote {path}");
}

#[test]
#[ignore = "requires a local 274 engine and LIVE=1"]
fn capture_bank_sweep() {
    if !live() {
        return;
    }

    let pixels = FrameBuf::new();
    let input = SlotInput::new();
    let (tx, rx) = mpsc::channel();
    input.connect_rx(rx);
    input.set_enabled(true);

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
        |c, _| {
            c.set_draw(true);
        },
    );

    wait_ingame(&play, 1, Duration::from_secs(90), "booth_capture");
    thread::sleep(Duration::from_secs(3));

    // Take the initial frame.
    let take = |idx: usize| {
        for _ in 0..40 {
            match pixels.take() {
                Some(FrameOutput::Texture(handle)) => {
                    let frame = handle.read_back();
                    save_ppm(&frame, &format!("/tmp/booth_{idx:02}.ppm"));
                    return;
                }
                Some(FrameOutput::PixMap(pix)) => {
                    save_ppm(&pix.pixels, &format!("/tmp/booth_{idx:02}.ppm"));
                    return;
                }
                None => thread::sleep(Duration::from_millis(100)),
            }
        }
        fail("no frame captured");
    };

    take(0);

    // Rotate left ~45 degrees per step (ArrowLeft ch=1), capture each step.
    for i in 1..8 {
        for _ in 0..3 {
            tx.send(InputEv::Key { down: true, ch: 1 }).unwrap();
            thread::sleep(Duration::from_millis(120));
            tx.send(InputEv::Key { down: false, ch: 1 }).unwrap();
            thread::sleep(Duration::from_millis(60));
        }
        thread::sleep(Duration::from_millis(300));
        take(i);
    }

    println!("PASS: sweep captured");
}
