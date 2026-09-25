// Catalog-run gold: every bright card remaps; dim names stay stamped;
// WalkTo is never a catalog card. Skips when `$RS2B0T` / persisted root
// is absent (CI without the clone).

use std::path::PathBuf;

use script::load::JsLibrary;
use script::ScriptSource;

mod common;

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

/// Registry/remap checks only validate module paths, not ESM named exports.
/// Keep an unconditional module-graph regression even on CI without the
/// external frozen catalog. The two changed cards import this exact surface.
#[test]
fn bank_picker_import_links_and_awaits_host_selection() {
    use api::named_banks::{NamedBank, NamedBankFacts};
    use api::snapshot::WorldTile;
    use script::isolate_fb::{BankSelectionInput, NativeFactsInput};
    use script::{LoadIsolate, LoadShape};
    use std::sync::Arc;
    let banks = Arc::new(NamedBankFacts::from_banks(vec![
        NamedBank::new(
            "Air near",
            WorldTile {
                x: 3224,
                z: 3218,
                level: 0,
            },
        ),
        NamedBank::new(
            "Walk near",
            WorldTile {
                x: 3200,
                z: 3210,
                level: 0,
            },
        ),
    ]));
    let iso = LoadIsolate::spawn_with_content(
        r#"
import { nearestBankReachable } from '../../api/bank/BankLocations.js';
import { Navigator } from '../../event/webwalk/Navigator.js';
export default class Probe extends LoopingBot {
    async loop() {
        globalThis.picked = await nearestBankReachable({ x: 3222, z: 3218, level: 0 }, Navigator);
    }
}
"#
        .into(),
        LoadShape::CompatClass,
        vec![],
        None,
        banks,
        Arc::new(api::run_policy::RunPolicyOverrideCell::new()),
    )
    .expect("Alcher/LeatherCrafter bank imports must link");
    let mut snapshot = common::ingame_snapshot();
    common::post_snapshot_input(&iso, &snapshot);
    iso.on_game_tick(1);
    let _ = iso.probe("true");
    let request_id = iso
        .drain_interacts()
        .into_iter()
        .find_map(|req| match req {
            script::shim::InteractReq::SelectBank { request_id, .. } => Some(request_id),
            _ => None,
        })
        .expect("reachable picker starts its native selection");
    for (tick, id) in [(2, request_id - 1), (3, request_id)] {
        snapshot.tick = tick;
        iso.post_snapshot(script::isolate_fb::encode_snapshot_with_native(
            &snapshot,
            NativeFactsInput {
                bank_selection: BankSelectionInput {
                    request_id: id,
                    generation: 1,
                    bank_index: 1,
                    kind: 2,
                },
                ..Default::default()
            },
        ));
        iso.on_game_tick(tick);
        let actual = iso.probe("typeof picked === 'undefined' ? null : [picked.name, picked.tile.x, picked.tile.z, picked.tile.level]").unwrap();
        if tick == 2 {
            assert_eq!(
                actual,
                serde_json::Value::Null,
                "a stale completion cannot settle the current selection"
            );
        } else {
            assert_eq!(
                actual,
                serde_json::json!(["Walk near", 3200, 3210, 0]),
                "the awaited result is the host's reachable bank, not a JS air-nearest substitute"
            );
        }
    }
    iso.join();
}

/// Link with the selected content just like Start. `prepare_card` alone
/// validates with no content and would turn drop-db consumers into false gaps.
fn catalog_graphs(
    root: &std::path::Path,
    label: &str,
) -> std::collections::BTreeMap<String, Result<(), String>> {
    let dir = scratch(label);
    let mut lib = JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache"));
    lib.register_rs2b0t(root, &dir.join("rs2b0t-path"))
        .expect("catalog register");
    let names: Vec<_> = lib
        .cards()
        .iter()
        .filter(|card| {
            card.source == ScriptSource::Catalog
                && !script::is_catalog_dim(&card.name)
                && !card
                    .unloadable
                    .as_deref()
                    .is_some_and(|reason| reason.starts_with("dim:"))
        })
        .map(|card| card.name.clone())
        .collect();
    let content = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
    let mut results = std::collections::BTreeMap::new();
    for name in names {
        let result = (|| {
            lib.ensure_js(ScriptSource::Catalog, &name)?;
            let card = lib.get(ScriptSource::Catalog, &name).unwrap();
            if let Some(reason) = &card.unloadable {
                return Err(reason.clone());
            }
            let siblings = script::resolve_sibling_modules(
                &card.path,
                &card.origin,
                lib.cache(),
                script::CacheMeta {
                    kind: script::ScriptKind::Compat,
                    source: ScriptSource::Catalog,
                    shape: Some(format!("{:?}", card.shape)),
                    api_family: None,
                },
            )?;
            let iso = script::LoadIsolate::spawn_with_game_data(
                card.js.clone(),
                card.shape,
                siblings,
                content.clone(),
            )?;
            // Spawn returns before V8 setup. Probe is a command-queue barrier,
            // not a tick: no gameplay runs just to establish LOAD.
            let probe = iso.probe("true");
            let ready = iso.poll_ready();
            iso.join();
            match ready {
                script::Ready::Ready => probe.map(|_| ()),
                script::Ready::Failed(error) => Err(error),
                script::Ready::Pending => Err("setup did not settle at the probe barrier".into()),
            }
        })();
        results.insert(name, result);
    }
    std::fs::remove_dir_all(dir).unwrap();
    assert!(!results.is_empty(), "frozen catalog has no bright cards");
    let failures: Vec<_> = results
        .iter()
        .filter_map(|(name, result)| {
            result
                .as_ref()
                .err()
                .map(|error| format!("{name}: {error}"))
        })
        .collect();
    eprintln!(
        "catalog {label} at {}: {} linked; failures:\n{}",
        root.display(),
        results.len() - failures.len(),
        failures.join("\n")
    );
    results
}

/// Both changed graphs must load. With RS2B0T_BASELINE, also protect every
/// old-pin loadable card; pre-existing unsupported graphs are not regressions.
#[test]
fn frozen_catalog_module_graphs_load() {
    let Some(root) = script::rs2b0t_root() else {
        eprintln!("SKIP frozen catalog link proof: RS2B0T is not configured");
        return;
    };
    let current = catalog_graphs(&root, "current-linked-graphs");
    for name in ["Alcher", "LeatherCrafter"] {
        assert_eq!(
            current.get(name),
            Some(&Ok(())),
            "{name} must load at the selected pin"
        );
    }
    let Some(baseline) = std::env::var_os("RS2B0T_BASELINE") else {
        eprintln!("SKIP old/new load differential: RS2B0T_BASELINE is not configured");
        return;
    };
    let previous = catalog_graphs(std::path::Path::new(&baseline), "previous-linked-graphs");
    let regressions: Vec<_> = previous
        .iter()
        .filter(|(_, result)| result.is_ok())
        .filter_map(|(name, _)| match current.get(name) {
            Some(Ok(())) => None,
            Some(Err(error)) => Some(format!("{name}: {error}")),
            None => Some(format!("{name}: missing or newly dim")),
        })
        .collect();
    assert!(
        regressions.is_empty(),
        "old-pin loadable cards regressed:\n{}",
        regressions.join("\n")
    );
}
