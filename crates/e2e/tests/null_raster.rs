//! Live: two slots, only `test` set_draw; `test2` must not enter game_draw.
//!
//! LIVE=1 cargo test -p e2e --test null_raster -- --ignored --test-threads=1 --nocapture

mod common;

use std::thread;
use std::time::Duration;

use common::{fail, live, options, profiles, wait_ingame};
use host_play::run_with_io;

#[test]
#[ignore = "requires a local 274 engine and LIVE=1"]
fn live_unfocused_never_enters_game_draw() {
    if !live() {
        return;
    }
    let play = run_with_io(
        &options(),
        profiles(&[("test", "test"), ("test2", "test2")]),
        |_| (None, None),
        |c, name| c.set_draw(name == "test"),
    );
    wait_ingame(&play, 2, Duration::from_secs(120), "null_raster");
    let snap = |name: &str| {
        play.statuses()
            .into_iter()
            .find(|s| s.username == name)
            .unwrap_or_else(|| fail(&format!("null_raster: missing {name}")))
    };
    let a0 = snap("test2");
    thread::sleep(Duration::from_secs(3));
    let a1 = snap("test2");
    let b1 = snap("test");
    println!(
        "null_raster test2 game_draw {}→{} title {}→{} bytes {}/{}",
        a0.game_draw_enters,
        a1.game_draw_enters,
        a0.title_screen_draw_enters,
        a1.title_screen_draw_enters,
        a1.bytes_in,
        a1.bytes_out
    );
    println!(
        "null_raster test  game_draw {} title {} bytes {}/{}",
        b1.game_draw_enters, b1.title_screen_draw_enters, b1.bytes_in, b1.bytes_out
    );
    if a1.game_draw_enters != a0.game_draw_enters {
        fail("null_raster: unfocused test2 game_draw_enters grew");
    }
    if a1.title_screen_draw_enters != a0.title_screen_draw_enters {
        fail("null_raster: unfocused test2 title_screen_draw_enters grew");
    }
    if b1.game_draw_enters == 0 {
        fail("null_raster: focused test never entered game_draw");
    }
}
