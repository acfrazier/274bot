fn parse_mode() -> panel::RunMode {
    let env = std::env::var("BOT_LIVE").ok();
    match panel::parse_live_args(std::env::args().skip(1), env.as_deref()) {
        Ok(mode) => mode,
        Err((code, msg)) => {
            eprintln!("{msg}");
            std::process::exit(code);
        }
    }
}

fn main() {
    if let Err(e) = panel::run_panel(parse_mode()) {
        eprintln!("panel: {e}");
        std::process::exit(1);
    }
}
