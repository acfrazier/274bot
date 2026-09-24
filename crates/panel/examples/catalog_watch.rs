//! Native panel entry for controlled catalog runs with isolated script stores.
//! All gameplay, rendering and scenario handling use the production panel.

fn main() {
    let mut args = match panel::parse_args(std::env::args().skip(1), None) {
        Ok(args) => args,
        Err((code, message)) => {
            eprintln!("{message}");
            std::process::exit(code);
        }
    };
    if !matches!(args.mode, panel::RunMode::Live(_)) {
        eprintln!("FAIL: catalog_watch requires --live script_<name>");
        std::process::exit(1);
    }
    args.catalog_core = true;
    let catalog_root = script::rs2b0t_env();
    let stores = script::IsolatedEnv::enter(&format!("catalog-watch-{}", std::process::id()));
    // Isolate mutable stores while retaining the explicitly selected catalog.
    if let Some(root) = catalog_root {
        stores.set_rs2b0t(&root);
    }
    if let Err(error) = panel::run_panel(args) {
        eprintln!("FAIL: catalog_watch: {error}");
        std::process::exit(1);
    }
}
