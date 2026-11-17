//! `tui-play` entry point: parse flags and run the interactive headless
//! panel or a `--live script_<name>` harness (PASS/FAIL from the scenario
//! runner, not a screenshot).

#[cfg(feature = "memory-profile")]
#[global_allocator]
static ALLOCATOR: frontend_core::process::BenchmarkAllocator =
    frontend_core::process::BENCHMARK_ALLOCATOR;

use std::process::ExitCode;

fn main() -> ExitCode {
    // `state preferences …` maintenance commands run before any frontend,
    // vault or Play startup, and exit.
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) =
        frontend_core::prefs::maintenance_command(frontend_core::prefs::StateFrontend::Tui, &argv)
    {
        return ExitCode::from(code as u8);
    }
    if let Err(error) = frontend_core::process::bootstrap() {
        api::host_log!(stderr; api::hostlog::Category::Lifecycle, api::hostlog::Level::Error, "{error}");
        return ExitCode::FAILURE;
    }
    // Before any slot starts: macOS defaults the soft limit to 256 open files.
    // The session log records the result when it opens.
    frontend_core::process::raise_fd_limit();
    let result = tui::bin::main();
    if let Err(error) = frontend_core::process::shutdown_background_work() {
        api::host_log!(stderr; api::hostlog::Category::Lifecycle, api::hostlog::Level::Error, "background work shutdown: {error}");
        ExitCode::FAILURE
    } else {
        result
    }
}
