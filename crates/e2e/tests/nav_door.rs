//! Live: the shared `nav_door` scenario run headlessly — the twin of
//! `panel-play --live script_nav_door`. The scenario is a fleet: profile 0
//! (`test`) is the driven walker (cheat-tele to the Catherby range-house
//! outside stand, then a whole-world `Follow` through the door), profile 1
//! (`test2`) is the closer companion that `op_loc`s the door shut on every
//! open tick. One scenario, two runners: the headed panel and this test
//! drive the same [`ScenarioRunner`], so both pass/fail identically. The
//! driven slot ticks the walker; the companion slot ticks its own
//! per-frame hook — a slammed door surfaces as the follow's terminal
//! outcome, which is the current diagnostic target.
//!
//! Run with the engine up:
//! `LIVE=1 cargo test -p e2e --test nav_door -- --ignored --test-threads=1 --nocapture`

mod common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{fail, live, mint_seed, options, profiles, wait_ingame};
use host_play::run_with_io;
use scenario::{RunnerStatus, ScenarioRunner};

#[test]
#[ignore = "requires a local 274 engine, nav pack, two accounts, and LIVE=1"]
fn nav_door() {
    if !live() {
        return;
    }

    let scenario = scenario::get("nav_door").expect("nav_door scenario in registry");
    let mainland = scenario.seed.mainland;
    let n = scenario.seed.profiles.len();
    let runner = Arc::new(Mutex::new(ScenarioRunner::new(scenario)));
    // The headless twin never writes shots: explicit no-op sink (the same
    // behavior as the runner's default). Both fleet slots get minted
    // per-run accounts — never the shared `test`/`test2` saves.
    let entries = {
        let mut r = runner.lock().unwrap();
        r.set_shot_sink(Box::new(|_, _| {}));
        mint_seed(&mut r, n)
    };
    let mut opts = options();
    opts.mainland = mainland;
    let play = run_with_io(&opts, profiles(&entries), |_| (None, None), {
        let runner = Arc::clone(&runner);
        move |c, name| {
            let mut r = runner.lock().unwrap();
            if r.drives(name) {
                r.tick(c);
            } else if let Some(index) = r.companion_for(name) {
                r.companion_tick(index, c);
            }
        }
    });
    runner.lock().unwrap().set_obj_names(play.obj_names());

    // The fleet gate: both slots ingame with scene 2 (the walker's own
    // mainland seed gate still decides when it may tele; the closer needs
    // its slot up before it can slam).
    wait_ingame(&play, 2, Duration::from_secs(150), "nav_door");

    // Poll the shared runner for a terminal status; a FAIL or a timeout
    // both exit 1 (rs2b0t harness style).
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        let (status, evidence) = {
            let r = runner.lock().unwrap();
            (r.status(), r.evidence().cloned())
        };
        let record = evidence.as_ref().map(|ev| ev.to_json()).unwrap_or_default();
        match status {
            RunnerStatus::Passed => {
                println!("PASS: nav_door {record}");
                return;
            }
            RunnerStatus::Failed(msg) => {
                eprintln!("FAIL: nav_door {record}");
                fail(&format!("nav_door: {msg}"));
            }
            other => {
                if Instant::now() >= deadline {
                    fail(&format!(
                        "nav_door: no terminal status within 180s ({other:?})"
                    ));
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        }
    }
}
