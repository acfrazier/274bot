//! Headed production-panel entry for the two-quest operator Skip proof.

fn main() {
    frontend_core::process::bootstrap().expect("protected V8 bootstrap on process main thread");
    let mut args = match panel::parse_args(std::env::args().skip(1), None) {
        Ok(args) => args,
        Err((code, message)) => {
            eprintln!("{message}");
            std::process::exit(code);
        }
    };
    if !matches!(&args.mode, panel::RunMode::Live(name) if name == "script_quester_skip") {
        eprintln!("FAIL: quester_skip requires --live script_quester_skip");
        std::process::exit(1);
    }
    args.catalog_core = true;
    let catalog_root = frontend_core::test_support::catalog_root();
    let stores = frontend_core::test_support::IsolatedStores::enter(&format!(
        "quester-skip-panel-{}",
        std::process::id()
    ));
    if let Some(root) = catalog_root {
        stores.set_catalog_root(&root);
    }
    if let Err(error) = panel::run_panel(args) {
        eprintln!("FAIL: quester_skip: {error}");
        std::process::exit(1);
    }
}
