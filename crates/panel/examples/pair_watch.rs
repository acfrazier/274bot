//! Native panel entry for controlled paired catalog runs.
//! Both slots stay visible. Ordinary panel-play leaves pair_core off.

fn main() {
    frontend_core::process::bootstrap().expect("protected V8 bootstrap on process main thread");
    let mut args = match panel::parse_args(std::env::args().skip(1), None) {
        Ok(args) => args,
        Err((code, message)) => {
            eprintln!("{message}");
            std::process::exit(code);
        }
    };
    let pair_core = matches!(
        &args.mode,
        panel::RunMode::Live(name)
            if matches!(
                name.as_str(),
                "script_nature_crafter_air"
                    | "script_mule_crafter_air"
                    | "script_flax_runner"
                    | "script_duel_arena"
            )
    );
    let allowed = pair_core
        || matches!(
            &args.mode,
            panel::RunMode::Live(name)
                if matches!(
                    name.as_str(),
                    "script_clue_duel_3554" | "script_jive_kq_four"
                )
        );
    if !allowed {
        eprintln!(
            "FAIL: pair_watch requires --live script_nature_crafter_air|script_mule_crafter_air|script_flax_runner|script_duel_arena|script_clue_duel_3554|script_jive_kq_four"
        );
        std::process::exit(1);
    }
    // The legacy pair cells use the native witness. The clue/Jive gold
    // cells are qualified by their multi-slot ScenarioRunner proofs.
    args.pair_core = pair_core;
    let catalog_root = frontend_core::test_support::catalog_root();
    let stores = frontend_core::test_support::IsolatedStores::enter(&format!(
        "pair-watch-{}",
        std::process::id()
    ));
    // Isolate mutable stores while retaining the explicitly selected catalog.
    if let Some(root) = catalog_root {
        stores.set_catalog_root(&root);
    }
    if let Err(error) = panel::run_panel(args) {
        eprintln!("FAIL: pair_watch: {error}");
        std::process::exit(1);
    }
}
