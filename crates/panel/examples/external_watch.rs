//! Native panel entry for the external raw TypeScript loader smoke.
//! Gameplay, rendering and session handling use the production panel.

fn main() {
    frontend_core::process::bootstrap().expect("protected V8 bootstrap on process main thread");
    let mut args = match panel::parse_args(std::env::args().skip(1), None) {
        Ok(args) => args,
        Err((code, message)) => {
            eprintln!("{message}");
            std::process::exit(code);
        }
    };
    let allowed = matches!(
        args.mode,
        panel::RunMode::Live(ref name) if name == frontend_core::live::external_loader_live_name()
    );
    if !allowed {
        eprintln!(
            "FAIL: external_watch requires --live {}",
            frontend_core::live::external_loader_live_name()
        );
        std::process::exit(1);
    }
    args.external_core = true;
    let _stores = frontend_core::test_support::IsolatedStores::enter(&format!(
        "external-watch-{}",
        std::process::id()
    ));
    if let Err(error) = panel::run_panel(args) {
        eprintln!("FAIL: external_watch: {error}");
        std::process::exit(1);
    }
}
