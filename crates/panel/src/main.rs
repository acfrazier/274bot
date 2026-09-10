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
    if let Err(e) = panel::run_panel(parse_args()) {
        eprintln!("panel: {e}");
        std::process::exit(1);
    }
}
