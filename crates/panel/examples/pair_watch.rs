//! Native panel entry for controlled paired catalog runs.
//! Both slots stay visible. Ordinary panel-play leaves pair_core off.

fn main() {
    let mut args = match panel::parse_args(std::env::args().skip(1), None) {
        Ok(args) => args,
        Err((code, message)) => {
            eprintln!("{message}");
            std::process::exit(code);
        }
    };
    if !matches!(args.mode, panel::RunMode::Live(_)) {
        eprintln!(
            "FAIL: pair_watch requires --live script_nature_crafter_air|script_mule_crafter_air|script_flax_runner"
        );
        std::process::exit(1);
    }
    args.pair_core = true;
    let _stores = script::IsolatedEnv::enter(&format!("pair-watch-{}", std::process::id()));
    if let Err(error) = panel::run_panel(args) {
        eprintln!("FAIL: pair_watch: {error}");
        std::process::exit(1);
    }
}
