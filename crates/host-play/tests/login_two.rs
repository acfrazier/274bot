//! Live: two headless slots share one `cache_dir`; both log in through the
//! FIFO queue and reach `ingame && scene_state == 2`. Grants are FIFO
//! (one head at a time) but not 2.5 s spaced — the engine has no
//! inter-grant delay.
//!
//! Run with the engine up: `LIVE=1 cargo test -p e2e -- --ignored`.

mod common;

use std::time::Duration;

use common::{fail, live, login_starts, options, profiles, wait_ingame};
use host_play::run;

#[test]
#[ignore = "requires a local 274 engine and LIVE=1"]
fn login_two() {
    if !live() {
        return;
    }
    let play = run(
        &options(),
        profiles(&[("test", "test"), ("test2", "test2")]),
    );
    wait_ingame(&play, 2, Duration::from_secs(150), "login_two");

    let starts = login_starts(&play.statuses());
    if starts.len() != 2 {
        fail(&format!(
            "login_two: expected 2 login handshakes, got {}",
            starts.len()
        ));
    }
    println!(
        "PASS: login_two: handshake gap {:?}",
        starts[1].duration_since(starts[0])
    );
}
