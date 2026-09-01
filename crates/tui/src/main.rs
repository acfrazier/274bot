//! `tui-play` entry point: parse flags and run the interactive headless
//! panel or a `--live script_<name>` harness (PASS/FAIL from the scenario
//! runner, not a screenshot).

use std::process::ExitCode;

fn main() -> ExitCode {
    tui::bin::main()
}
