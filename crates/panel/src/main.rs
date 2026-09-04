fn parse_mode() -> panel::RunMode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.iter().any(|a| a == "--prod") {
        client::set_bot_target(client::BotTarget::Prod);
    }
    let env = std::env::var("BOT_LIVE").ok();
    match panel::parse_live_args(argv, env.as_deref()) {
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
