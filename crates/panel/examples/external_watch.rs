//! Native panel entry for the external raw TypeScript loader smoke.
//! Gameplay, rendering and session handling use the production panel.

fn main() {
    let mut args = match panel::parse_args(std::env::args().skip(1), None) {
        Ok(args) => args,
        Err((code, message)) => {
            eprintln!("{message}");
            std::process::exit(code);
        }
    };
    let allowed = matches!(
        args.mode,
        panel::RunMode::Live(ref name) if name == host_play::external_loader::LIVE_NAME
    );
    if !allowed {
        eprintln!(
            "FAIL: external_watch requires --live {}",
            host_play::external_loader::LIVE_NAME
        );
        std::process::exit(1);
    }
    args.external_core = true;
    let _stores = script::IsolatedEnv::enter(&format!("external-watch-{}", std::process::id()));
    if let Err(error) = panel::run_panel(args) {
        eprintln!("FAIL: external_watch: {error}");
        std::process::exit(1);
    }
}
