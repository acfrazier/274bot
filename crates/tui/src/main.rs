//! `tui-play` entry point: parse flags and run the interactive headless
//! panel or a `--live script_<name>` harness (PASS/FAIL from the scenario
//! runner, not a screenshot).

#[cfg(feature = "memory-profile")]
#[global_allocator]
static ALLOCATOR: frontend_core::process::BenchmarkAllocator =
    frontend_core::process::BENCHMARK_ALLOCATOR;

use std::process::ExitCode;

fn main() -> ExitCode {
    #[cfg(feature = "luau-probe")]
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--luau-probe")) {
        return luau_probe::run_cli();
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
