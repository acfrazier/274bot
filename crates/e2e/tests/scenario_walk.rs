#![cfg(feature = "scenario-luau")]

//! Live File-runner E2E for the registered `walk` scenario.
//!
//! Run with the engine up and the R289 nav pack at the standard path:
//! `LIVE=1 SCENARIO_ID=walk cargo test -p e2e --no-default-features --features scenario-luau --test scenario_walk -- --ignored --test-threads=1 --nocapture`

mod common;

use common::file_scenario::{registered_options, run};

#[test]
#[ignore = "requires a selected Local profile and a live engine"]
fn scenario_walk() -> Result<(), String> {
    run(registered_options("walk", "courtyard")?, false)
}
