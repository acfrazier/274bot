#[cfg(feature = "memory-profile")]
#[global_allocator]
static ALLOCATOR: host_play::memory::BenchmarkAllocator = host_play::memory::BENCHMARK_ALLOCATOR;

fn parse_args() -> panel::PanelArgs {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let env = std::env::var("BOT_LIVE").ok();
    match panel::parse_args(argv, env.as_deref()) {
        Ok(args) => args,
        Err((code, msg)) => {
            eprintln!("{msg}");
            std::process::exit(code);
        }
    }
}

fn main() -> std::process::ExitCode {
    script::bootstrap_v8_platform().expect("protected V8 bootstrap on process main thread");
    #[cfg(feature = "memory-profile")]
    host_play::memory::mark_process_start();
    let args = parse_args();
    // Before any slot starts: a Finder-launched app gets 256 open files.
    // The session log records the result when it opens.
    host_play::fd_limit::raise_at_startup();
    host_play::passphrase::warn_legacy_env("panel-play");
    let run = panel::run_panel(args);
    let shutdown = script::work_admission::WorkService::shutdown_if_started();
    if let Err(error) = &run {
        api::host_log!(stderr; api::hostlog::Category::Lifecycle, api::hostlog::Level::Error, "panel: {error}");
    }
    if let Err(error) = &shutdown {
        api::host_log!(stderr; api::hostlog::Category::Lifecycle, api::hostlog::Level::Error, "background work shutdown: {error}");
    }
    if run.is_err() || shutdown.is_err() {
        std::process::ExitCode::FAILURE
    } else {
        std::process::ExitCode::SUCCESS
    }
}
