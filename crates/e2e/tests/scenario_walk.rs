//! Live: the shared `walk` scenario run headlessly — the twin of
//! `panel-play --live script_walk`. One scenario, two runners: the
//! headed panel and this test drive the same `ScenarioRunner`, so both
//! pass/fail identically. PASS only when the mainland player actually
//! arrives at the courtyard tile (no "Running poll" stub).
//!
//! Run with the engine up and the nav pack at the standard path:
//! `LIVE=1 cargo test -p e2e --test scenario_walk -- --ignored --test-threads=1 --nocapture`

mod common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{fail, live, options, profiles};
use host_play::run_with_io;
use scenario::{RunnerStatus, ScenarioRunner};

#[test]
#[ignore = "requires a local 274 engine, nav pack, and LIVE=1"]
fn scenario_walk() {
    if !live() {
        return;
    }

    let scenario = scenario::get("walk").expect("walk scenario in registry");
    let mainland = scenario.seed.mainland;
    let seed_profiles = scenario.seed.profiles.clone();
    let runner = Arc::new(Mutex::new(ScenarioRunner::new(scenario)));
    // The headless twin never writes shots: explicit no-op sink (the same
    // behavior as the runner's default).
    runner
        .lock()
        .unwrap()
        .set_shot_sink(Box::new(|_, _| {}));
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

    // Poll the shared runner for a terminal status; a FAIL or a timeout
    // both exit 1 (rs2b0t harness style).
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        let (status, evidence) = {
            let r = runner.lock().unwrap();
            (r.status(), r.evidence().cloned())
        };
        let record = evidence
            .as_ref()
            .map(|ev| ev.to_json())
            .unwrap_or_default();
        match status {
            RunnerStatus::Passed => {
                println!("PASS: scenario walk {record}");
                return;
            }
            RunnerStatus::Failed(msg) => {
                eprintln!("FAIL: scenario walk {record}");
                fail(&format!("scenario walk: {msg}"));
            }
            other => {
                if Instant::now() >= deadline {
                    fail(&format!(
                        "scenario walk: no terminal status within 180s ({other:?})"
                    ));
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        }
    }
}
