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

    // Auto-run send proof: if energy is already ≥20 at scene 2, skip as the
    // plan allows (fresh accounts on the local engine often spawn full).
    // If energy is below then crosses, assert exactly one 153 send.
    let s = play.statuses()[0].clone();
    if s.runenergy >= 20 {
        println!(
            "skip auto-run assert: energy already {} (run_sends={})",
            s.runenergy, s.run_sends
        );
        return;
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        let s = play.statuses()[0].clone();
        if s.runenergy >= 20 {
            if s.run_sends != 1 {
                fail(&format!(
                    "login_one auto-run: energy crossed 20 but run_sends={} (want 1)",
                    s.run_sends
                ));
            }
            println!(
                "PASS: login_one auto-run sent once at energy {}",
                s.runenergy
            );
            return;
        }
        if std::time::Instant::now() >= deadline {
            fail(&format!(
                "login_one auto-run: energy stayed {} (never crossed 20); run_sends={}",
                s.runenergy, s.run_sends
            ));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}
