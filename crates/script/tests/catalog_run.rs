// Catalog-run gold: every bright card remaps; dim names stay stamped;
// WalkTo is never a catalog card. Skips when `$RS2B0T` / persisted root
// is absent (CI without the clone).

use std::path::PathBuf;

use script::load::JsLibrary;
use script::ScriptSource;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "274bot-catalog-run-{}-{}",
        std::process::id(),
        name
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Paths the plan never registers. Cards that first-import these stay
/// Start-refused without being name-locked.
fn locked_unloadable(spec: &str) -> bool {
    spec.contains("WalkExecutor.js")
        || spec.contains("event/webwalk/Navigator.js")
        || spec.contains("ToolAcquire.js")
        || spec.contains("/defs/")
        || spec.contains("barcrawl/")
}

#[test]
fn catalog_cards_except_dim_set_remap() {
    let Some(root) = script::rs2b0t_root() else {
        return;
    };
    let dir = scratch("gold");
    let mut lib = JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache"));
    lib.register_rs2b0t(&root, &dir.join("rs2b0t-path"))
        .expect("catalog register");

    let mut leftover = Vec::new();
    let mut dim_ok = 0usize;
    let mut bright_ok = 0usize;
    for card in lib.cards() {
        assert_ne!(
            card.name.as_str(),
            "WalkTo",
            "WalkTo stays reserved, never a catalog card"
        );
        if script::is_catalog_dim(&card.name) {
            assert!(
                card.unloadable.is_some(),
                "{} is dim but unloadable is None",
                card.name
            );
            dim_ok += 1;
            continue;
        }
        if card.source != ScriptSource::Catalog {
            continue;
        }
        match &card.unloadable {
            None => bright_ok += 1,
            Some(u) if locked_unloadable(u) => {}
            Some(u) => leftover.push(format!("{}: {u}", card.name)),
        }
    }
    assert!(
        leftover.is_empty(),
        "bright catalog cards still unloadable ({bright_ok} ok, {dim_ok} dim):\n{}",
        leftover.join("\n")
    );
    assert!(dim_ok > 0, "dim names must still be present in the catalog");
}
