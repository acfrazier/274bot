//! Isolated interactive entry for native startup proof; product code unchanged.
fn main() {
    let args = panel::parse_args(std::env::args().skip(1), None).unwrap_or_else(|(code, error)| {
        eprintln!("{error}"); std::process::exit(code)
    });
    let _stores = script::IsolatedEnv::enter(&format!("startup-watch-{}", std::process::id()));
    if let Err(error) = panel::run_panel(args) {
        eprintln!("FAIL: startup_watch: {error}"); std::process::exit(1);
    }
}
