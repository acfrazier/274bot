#[cfg(feature = "memory-profile")]
#[global_allocator]
static ALLOCATOR: frontend_core::process::BenchmarkAllocator =
    frontend_core::process::BENCHMARK_ALLOCATOR;

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
    frontend_core::process::bootstrap().expect("protected V8 bootstrap on process main thread");
    let args = parse_args();
    // Before any slot starts: a Finder-launched app gets 256 open files.
    // The session log records the result when it opens.
    frontend_core::process::raise_fd_limit();
    frontend_core::launch::warn_legacy_passphrase_env("panel-play");
    let run = panel::run_panel(args);
    let shutdown = frontend_core::process::shutdown_background_work();
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
