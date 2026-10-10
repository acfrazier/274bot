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

fn main() {
    #[cfg(feature = "memory-profile")]
    host_play::memory::mark_process_start();
    let args = parse_args();
    // Before any slot starts: a Finder-launched app gets 256 open files.
    if let Some(line) = host_play::fd_limit::describe(&host_play::fd_limit::raise_open_file_limit())
    {
        use frontend_core::log::{Level, Source};
        frontend_core::log::global().process_line(Source::Host, Level::Info, &line);
    }
    host_play::passphrase::warn_legacy_env("panel-play");
    if let Err(e) = panel::run_panel(args) {
        eprintln!("panel: {e}");
        std::process::exit(1);
    }
}
