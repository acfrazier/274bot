//! Live capture: log in test/test near the bank, rotate the camera, and
//! save a 360-degree sweep of the 3D viewport as PPMs for inspection.
//!
//! `LIVE=1 cargo test -p e2e --test booth_capture -- --ignored --test-threads=1 --nocapture`

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use api::snapshot::GameSnapshot;
use client::render::backend::FrameOutput;
use common::{fail, live, options, profiles, wait_ingame, wait_logged_out};
use host::{FrameBuf, InputEv, SlotInput};
use host_play::run_with_io;

const W: usize = 765;
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
        |c, _, _hold| {
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

fn angle_name(a: i32) -> &'static str {
    match a {
        0 => "WEST",
        1 => "NORTH",
        2 => "EAST",
        3 => "SOUTH",
        _ => "?",
    }
}

fn shape_name(s: i32) -> &'static str {
    match s {
        0 => "WALL_STRAIGHT",
        1 => "WALL_DIAGONAL_CORNER",
        2 => "WALL_L",
        3 => "WALL_SQUARE_CORNER",
        9 => "WALL_DIAGONAL",
        10 => "CENTREPIECE_STRAIGHT",
        11 => "CENTREPIECE_DIAGONAL",
        _ => "other",
    }
}

/// Live: dump loc 1602 / bank-booth tiles around the player, then IF-logout
/// so the engine does not 60s-lock the account.
///
/// `LIVE=1 cargo test -p e2e --test booth_capture dump_bank_locs -- --ignored --test-threads=1 --nocapture`
#[test]
#[ignore = "requires a local 274 engine and LIVE=1"]
fn dump_bank_locs() {
    if !live() {
        return;
    }

    let dumped = Arc::new(AtomicBool::new(false));
    let dumped_flag = Arc::clone(&dumped);
    let mut play = run_with_io(
        &options(),
        profiles(&[("test", "test")]),
        |_| (None, None),
        move |c, _, _hold| {
            c.set_draw(true);
            if !(c.ingame && c.scene_state == 2) {
                return;
            }
            if dumped_flag.swap(true, Ordering::Relaxed) {
                return;
            }
            let mut snap = GameSnapshot::new();
            snap.rebuild(c);
            let me = snap.tile();
            eprintln!("player tile={me:?}");
            let mut walls = Vec::new();
            let mut booths = Vec::new();
            for loc in snap.locs() {
                let keep = loc.id == 1602
                    || loc.id == 2213
                    || loc.id == 2214
                    || loc.id == 2215
                    || loc.id == 961;
                if !keep {
                    continue;
                }
                if let Some((px, pz, _)) = me {
                    let dx = (loc.tile.x - px).abs();
                    let dz = (loc.tile.z - pz).abs();
                    if dx > 20 || dz > 20 {
                        continue;
                    }
                }
                eprintln!(
                    "loc id={} name={:?} tile=({},{},{}) shape={} ({}) angle={} ({}) layer={:?} dist={}",
                    loc.id,
                    loc.name,
                    loc.tile.x,
                    loc.tile.z,
                    loc.tile.level,
                    loc.shape,
                    shape_name(loc.shape),
                    loc.angle,
                    angle_name(loc.angle),
                    loc.layer,
                    loc.distance
                );
                if loc.id == 1602 {
                    walls.push((loc.tile.x, loc.tile.z, loc.tile.level, loc.shape, loc.angle));
                }
                if loc.id == 2213 || loc.id == 2214 || loc.id == 2215 {
                    booths.push((
                        loc.id,
                        loc.tile.x,
                        loc.tile.z,
                        loc.tile.level,
                        loc.shape,
                        loc.angle,
                    ));
                }
            }
            eprintln!("booths={} walls_1602={}", booths.len(), walls.len());
            for (bid, bx, bz, bl, bshape, bangle) in &booths {
                let same: Vec<_> = walls
                    .iter()
                    .filter(|(wx, wz, wl, _, _)| *wx == *bx && *wz == *bz && *wl == *bl)
                    .copied()
                    .collect();
                let west = walls.iter().any(|(wx, wz, wl, sh, an)| {
                    *wl == *bl
                        && *sh == 0
                        && (*an == 2 || *an == 0)
                        && *wz == *bz
                        && (*wx == *bx - 1 || *wx == *bx)
                });
                let south = walls.iter().any(|(wx, wz, wl, sh, an)| {
                    *wl == *bl
                        && *sh == 0
                        && *an == 3
                        && *wx == *bx
                        && (*wz == *bz || *wz == *bz - 1)
                });
                eprintln!(
                    "booth {bid} @({bx},{bz},{bl}) shape={bshape} angle={} same_tile_1602={same:?} west_adj_or_same={west} south_adj_or_same={south}",
                    angle_name(*bangle)
                );
            }
        },
    );

    wait_ingame(&play, 1, Duration::from_secs(90), "dump_bank_locs");
    // Let one rebuild land in the hook.
    thread::sleep(Duration::from_secs(2));
    if !dumped.load(Ordering::Relaxed) {
        eprintln!("WARN: loc dump did not run (scene never 2 in the hook)");
    }

    if let Some(arm) = play.arm("test") {
        arm.want_logout.store(true, Ordering::Relaxed);
    } else {
        fail("dump_bank_locs: no arm for test slot; cannot IF-logout");
    }
    wait_logged_out(&play, 1, Duration::from_secs(30), "dump_bank_locs");
    play.stop_slot("test");
    println!("PASS: dump_bank_locs (logged out)");
}
