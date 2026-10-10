//! `tui-play` entry point: parse flags and run the interactive headless
//! panel or a `--live script_<name>` harness (PASS/FAIL from the scenario
//! runner, not a screenshot).

#[cfg(feature = "memory-profile")]
#[global_allocator]
static ALLOCATOR: host_play::memory::BenchmarkAllocator = host_play::memory::BENCHMARK_ALLOCATOR;

use std::process::ExitCode;

fn main() -> ExitCode {
    #[cfg(feature = "memory-profile")]
    host_play::memory::mark_process_start();
    // Before any slot starts: macOS defaults the soft limit to 256 open files.
    // The session log records the result when it opens.
    host_play::fd_limit::raise_at_startup();
    tui::bin::main()
}
