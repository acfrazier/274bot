//! Live: one headless client via host-play, login queue, then
//! `ingame && scene_state == 2`.
//!
//! Run with the engine up: `LIVE=1 cargo test -p e2e -- --ignored`.

mod common;

use std::time::Duration;

use common::{fail, live, options, profiles, wait_ingame};
use host_play::run;

#[test]
#[ignore = "requires a local 274 engine and LIVE=1"]
fn login_one() {
    if !live() {
        return;
    }
    let play = run(&options(), profiles(&[("test", "test")]));
    // 90 s: nominal login+scene 2 is ~35 s; the extra headroom covers one
    // engine "account in use" (code 5) retry after a previous run's session.
    wait_ingame(&play, 1, Duration::from_secs(90), "login_one");
    if !play.statuses()[0].ingame {
        fail("login_one: slot reported not ingame");
    }
}
