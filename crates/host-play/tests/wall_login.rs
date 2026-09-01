//! Live: the wall path proven headless — two armed slots sit on the title
//! (no handshake, like a wall `load` with auto-login off), Login all
//! FIFO-serializes them into `ingame && scene_state == 2`, Logout all
//! puts both back on the title with a clean IF logout (a dirty disconnect
//! would hold the engine's 60 s "account in use" window and FAIL this
//! step), and a second Login all re-enters the FIFO.
//!
//! `!ingame` here is the client's local title state after the IF logout
//! press — this harness does not separately prove clean-vs-dirty TCP.
//!
//! Run with the engine up:
//! `LIVE=1 cargo test -p e2e --test wall_login -- --ignored --test-threads=1`

mod common;

use std::sync::atomic::Ordering;
use std::time::Duration;

use common::{assert_none_ingame_for, fail, live, options, profiles, wait_ingame, wait_logged_out};
use host_play::{run_with_io, SlotArm};

#[test]
#[ignore = "requires a local 274 engine and LIVE=1"]
fn wall_login_fifo_logout_all() {
    if !live() {
        return;
    }

    // Two armed slots on a shared login FIFO, no dear-app window. The
    // `want_login = false` arm holds each slot on the title like a wall
    // load; Login all flips it later.
    let mut play = run_with_io(&options(), vec![], |_| (None, None), |_, _, _| {});
    for p in profiles(&[("test", "test"), ("test2", "test2")]) {
        play.spawn_slot(p.clone(), None, None, Some(SlotArm::new(p.uid, false)));
    }
    let arm_a = play.arm("test").expect("test slot arm");
    let arm_b = play.arm("test2").expect("test2 slot arm");

    // The wall load hold: neither slot handshakes on its own.
    assert_none_ingame_for(&play, Duration::from_secs(3), "wall_login");

    // Login all: both arms want a login. The FIFO gate is racy — the
    // first requester is granted immediately and its handshake starts,
    // while the other may flash `queue_position == 1` for a 20 ms poll
    // (a 2-of-2 only appears if both enqueue before the first grant), so
    // poll until one slot shows either, then both must reach scene 2.
    arm_a.want_login.store(true, Ordering::Relaxed);
    arm_b.want_login.store(true, Ordering::Relaxed);
    let fifo_deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        let statuses = play.statuses();
        if statuses
            .iter()
            .any(|s| s.queue_position == 1 || s.login_started.is_some())
        {
            println!("PASS: wall_login FIFO observed: {statuses:?}");
            break;
        }
        if std::time::Instant::now() >= fifo_deadline {
            fail(&format!(
                "wall_login: FIFO never observed (no queue_position 1, no \
                 login_started) after 60 s; statuses: {statuses:?}"
            ));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    wait_ingame(&play, 2, Duration::from_secs(120), "wall_login");

    // Logout all: both arms press the clean IF logout. The slot threads
    // record the title state as soon as the 20 ms body exits; a slot
    // still ingame after 30 s means the logout never completed.
    arm_a.want_logout.store(true, Ordering::Relaxed);
    arm_b.want_logout.store(true, Ordering::Relaxed);
    wait_logged_out(&play, 2, Duration::from_secs(30), "wall_login");

    // Login all again: the logout latched both arms, so clear the latches
    // and re-arm. Both re-enter the FIFO and reach scene 2 again.
    arm_a.latch.store(false, Ordering::Relaxed);
    arm_b.latch.store(false, Ordering::Relaxed);
    arm_a.want_login.store(true, Ordering::Relaxed);
    arm_b.want_login.store(true, Ordering::Relaxed);
    wait_ingame(&play, 2, Duration::from_secs(120), "wall_login");

    // Stop both slot threads so the process can exit cleanly.
    arm_a.stop.store(true, Ordering::Relaxed);
    arm_b.stop.store(true, Ordering::Relaxed);
}
