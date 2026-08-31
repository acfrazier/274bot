//! Live: the shared `nav_full` scenario run headlessly — the twin of
//! `panel-play --live nav_full`. One scenario, two runners: the headed
//! panel and this test drive the same `ScenarioRunner`, so both
//! pass/fail identically. The scenario seeds the mainland hop, arms
//! `find` over the live scene's collision + the transport graph derived
//! from the baked whole-world pack, and drives `Traveller::follow` per
//! tick to the cross-mapsquare destination (3220, 3264, 0). PASS only
//! when the runner reports `Passed` (the `arrived` proof held); FAIL +
//! exit 1 on a runner failure or the outer timeout.
//!
//! Run with the engine up and the nav pack at the standard path:
//! `LIVE=1 cargo test -p e2e --test nav_full -- --ignored --test-threads=1 --nocapture`

mod common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{fail, live, options, profiles, wait_ingame};
use host_play::run_with_io;
use scenario::{RunnerStatus, ScenarioRunner};

/// The whole run's wall-clock backstop: the mainland seed can take ~90s
/// and the ~150-tile walk another ~2 minutes. The runner's deadline is
/// `settings.deadline` (360s for `nav_full`); 400s is the process
/// backstop.
const RUN_DEADLINE: Duration = Duration::from_secs(400);

#[test]
#[ignore = "requires a local 274 engine, nav pack, and LIVE=1"]
fn nav_full() {
    if !live() {
        return;
    }

    let scenario = scenario::get("nav_full").expect("nav_full scenario in registry");
    let mainland = scenario.seed.mainland;
    let seed_profiles = scenario.seed.profiles.clone();
    let runner = Arc::new(Mutex::new(ScenarioRunner::new(scenario)));
    {
        let mut r = runner.lock().unwrap();
        // The headless twin never writes shots: explicit no-op sink (the
        // same behavior as the runner's default). Runner deadline is
        // `settings.deadline` (360s); 400s is the process backstop.
        r.set_shot_sink(Box::new(|_, _| {}));
    }
    let mut opts = options();
    opts.mainland = mainland;
    let play = run_with_io(&opts, profiles(&seed_profiles), |_| (None, None), {
        let runner = Arc::clone(&runner);
        move |c, name| {
            let mut r = runner.lock().unwrap();
            if r.drives(name) {
                r.tick(c);
            }
        }
    });
    runner.lock().unwrap().set_obj_names(play.obj_names());

    // The mainland gate: wait for `ingame && scene_state == 2` (the
    // tutorial island also reaches scene 2, so the runner's own seed gate
    // — a mainland build base plus the world-bounds check — still decides
    // when the route may arm).
    wait_ingame(&play, 1, Duration::from_secs(90), "nav_full");

    // Poll the shared runner for a terminal status; a FAIL or a timeout
    // both exit 1 (rs2b0t harness style).
    let deadline = Instant::now() + RUN_DEADLINE;
    loop {
        let (status, evidence) = {
            let r = runner.lock().unwrap();
            (r.status(), r.evidence().cloned())
        };
        let record = evidence.as_ref().map(|ev| ev.to_json()).unwrap_or_default();
        match status {
            RunnerStatus::Passed => {
                println!("PASS: nav_full {record}");
                return;
            }
            RunnerStatus::Failed(msg) => {
                eprintln!("FAIL: nav_full {record}");
                fail(&format!("nav_full: {msg}"));
            }
            other => {
                if Instant::now() >= deadline {
                    fail(&format!(
                        "nav_full: no terminal status within {RUN_DEADLINE:?} ({other:?})"
                    ));
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        }
    }
}
