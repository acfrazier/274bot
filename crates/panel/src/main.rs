fn parse_live() -> Option<String> {
    let env = std::env::var("BOT_LIVE").ok();
    match panel::parse_live_args(std::env::args().skip(1), env.as_deref()) {
        Ok(v) => v,
        Err((code, msg)) => {
            eprintln!("{msg}");
            std::process::exit(code);
        }
    }
}

fn main() {
    if let Err(e) = panel::run_panel(parse_live()) {
        eprintln!("panel: {e}");
        std::process::exit(1);
    }
}
