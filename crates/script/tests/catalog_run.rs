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
fn new_clue_duel_modules_import_but_refuse_unwired_actions() {
    use script::{LoadIsolate, LoadShape};

    let src = r#"
import { ClueDuelHelper } from '../../api/duel/ClueDuel.js';
import { Duel } from '../../api/duel/Duel.js';
import { openClueBank } from '../../api/ai/clues/bankAccess.js';
import { walkAcrossClueDuel } from '../../api/ai/clues/duelTravel.js';
export default class T extends LoopingBot {
    loop() {
        const refuses = (fn) => {
            try { fn(); return 'unexpected success'; }
            catch (error) { return String(error); }
        };
        globalThis.__probe = JSON.stringify([
            refuses(() => new ClueDuelHelper('partner', () => {})),
            refuses(() => Duel.active()),
            refuses(() => openClueBank()),
            refuses(() => walkAcrossClueDuel()),
        ]);
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![])
        .expect("unwired duel module imports load");
    iso.on_game_tick(1);
    let value: Vec<String> = serde_json::from_str(
        iso.probe("__probe")
            .expect("duel refusal probe")
            .as_str()
            .unwrap(),
    )
    .unwrap();
    iso.join();
    for (actual, suffix) in value.iter().zip([
        "ClueDuel.ClueDuelHelper",
        "Duel.active",
        "bankAccess.openClueBank",
        "duelTravel.walkAcrossClueDuel",
    ]) {
        assert_eq!(actual, &format!("Error: not impl: {suffix}"));
    }
}

#[test]
fn autofighter_and_herb_cleaner_import_herbs_without_unloadable_stamp() {
    let Some(root) = script::rs2b0t_root() else {
        return;
    };
    let dir = scratch("herb-cards");
    let mut lib = JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache"));
    lib.register_rs2b0t(&root, &dir.join("rs2b0t-path"))
        .expect("catalog register");
    for name in ["AutoFighter", "HerbCleaner"] {
        let card = lib
            .get(ScriptSource::Catalog, name)
            .unwrap_or_else(|| panic!("{name} listed"));
        assert_eq!(
            card.unloadable, None,
            "{name} must load after herbs.js remap (not gameplay-qualified)"
        );
    }
}

#[test]
fn melee_partner_trade_and_type_only_imports_resolve() {
    let Some(root) = script::rs2b0t_root() else {
        return;
    };
    let dir = scratch("blocked-cards");
    let mut lib = JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache"));
    lib.register_rs2b0t(&root, &dir.join("rs2b0t-path"))
        .expect("catalog register");
    let unloadable = |name: &str| {
        lib.get(ScriptSource::Catalog, name)
            .unwrap_or_else(|| panic!("{name} listed"))
            .unloadable
            .clone()
    };
    // BrimhavenMossGiants names dangerZones.js only in `import type`.
    assert_eq!(unloadable("BrimhavenMossGiants"), None);
    assert_eq!(unloadable("JiveMarketDumper"), None);
    let dragons = unloadable("JiveDragons").unwrap_or_default();
    assert!(!dragons.contains("meleeWeapons.js"), "{dragons}");
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
        if script::is_catalog_dim(&card.name)
            || card
                .unloadable
                .as_deref()
                .is_some_and(|reason| reason.starts_with("dim:"))
        {
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
