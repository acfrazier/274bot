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

#[cfg(all(feature = "memory-profile", feature = "render-diagnostics"))]
fn run_with_calibration(args: panel::PanelArgs) -> Result<(), panel::window::PanelError> {
    let configured = frontend_core::bench::configure_render_calibration()
        .map_err(|error| panel::window::PanelError::LiveHarnessFailure(error.to_string()))?;
    if !configured {
        return panel::run_panel(args);
    }
    let redraw = match std::env::var("T44_HEADED_REDRAW").as_deref() {
        Ok("normal") => panel::runner_config().redraw,
        Ok("wait") => panel::window::RedrawMode::Wait,
        _ => {
            return Err(panel::window::PanelError::LiveHarnessFailure(
                "calibration requires T44_HEADED_REDRAW=normal or wait".into(),
            ));
        }
    };
    panel::run_panel_calibrated(args, redraw)
}

fn main() -> std::process::ExitCode {
    frontend_core::process::bootstrap().expect("protected V8 bootstrap on process main thread");
    let args = parse_args();
    // Before any slot starts: a Finder-launched app gets 256 open files.
    // The session log records the result when it opens.
    frontend_core::process::raise_fd_limit();
    frontend_core::launch::warn_legacy_passphrase_env("panel-play");
    #[cfg(all(feature = "memory-profile", feature = "render-diagnostics"))]
    let run = run_with_calibration(args);
    #[cfg(not(all(feature = "memory-profile", feature = "render-diagnostics")))]
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
