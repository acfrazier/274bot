//! Synthetic mini-catalog regressions: bright Start, module-graph load, and
//! import-remap stamping (no `$RS2B0T` / persisted rs2b0t-path).
//! Bright Start supplies selected local R289 facts to native callbacks.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use script::native::snapshot_source::StatInput;
use script_js::{JsLibrary, LoadIsolate};
use {script::ScriptKind, script::ScriptSource, script_js::CacheMeta, script_js::JsCache};

mod common;
#[path = "common/local_api_asset.rs"]
mod local_api_asset;
#[path = "common/local_game_data.rs"]
mod local_game_data;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "274bot-catalog-bright-{}-{}",
        std::process::id(),
        name
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_fixture_catalog(root: &Path) {
    let scripts = root.join("src/bot/scripts");
    let body = "import { Game } from '../../api/game/Game.js'; export default class T extends LoopingBot { loop() {} }";
    let herbs_card = r#"
import { HERBS } from '../../data/herbs.js';
export default class HerbCard extends LoopingBot {
    loop() { globalThis.__n = HERBS.length; }
}
"#;
    let type_only = r#"
import { bestMeleeWeapon } from '../../api/combat/meleeWeapons.js';
import type { DangerZoneRect } from '../../event/webwalk/data/dangerZones.js';
const _zone: DangerZoneRect = { minX: 0, maxX: 1, minZ: 0, maxZ: 1 };
export default class TypeOnly extends LoopingBot { loop() { bestMeleeWeapon([], { attack: 1 }); } }
"#;
    std::fs::create_dir_all(scripts.join("CookBot")).unwrap();
    std::fs::create_dir_all(scripts.join("Woodcutter")).unwrap();
    std::fs::create_dir_all(scripts.join("HerbCard")).unwrap();
    std::fs::create_dir_all(scripts.join("TypeOnly")).unwrap();
    std::fs::write(scripts.join("CookBot/CookBot.ts"), body).unwrap();
    std::fs::write(scripts.join("Woodcutter/Woodcutter.ts"), body).unwrap();
    std::fs::write(scripts.join("HerbCard/HerbCard.ts"), herbs_card).unwrap();
    std::fs::write(scripts.join("TypeOnly/TypeOnly.ts"), type_only).unwrap();
    std::fs::write(
        scripts.join("index.ts"),
        r#"
import CookBot from './CookBot/CookBot.js';
import Woodcutter from './Woodcutter/Woodcutter.js';
import HerbCard from './HerbCard/HerbCard.js';
import TypeOnly from './TypeOnly/TypeOnly.js';
ScriptRegistry.register({ name: 'CookBot', create: () => new CookBot() });
ScriptRegistry.register({ name: 'Woodcutter', create: () => new Woodcutter() });
ScriptRegistry.register({ name: 'HerbCard', create: () => new HerbCard() });
ScriptRegistry.register({ name: 'TypeOnly', create: () => new TypeOnly() });
"#,
    )
    .unwrap();
}

fn register_fixture(dir: &Path) -> JsLibrary {
    let root = dir.join("rs2b0t");
    write_fixture_catalog(&root);
    let mut lib = JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache"));
    lib.register_rs2b0t(&root, &dir.join("rs2b0t-path"))
        .expect("fixture catalog registers");
    lib
}

fn throw_shaped(line: &str) -> bool {
    line.contains("not impl") || line.contains("Error:") || line.contains("TypeError")
}

#[test]
fn synthetic_bright_catalog_cards_stay_unloadable_clean() {
    let dir = scratch("stamp");
    let lib = register_fixture(&dir);
    let mut bright = 0usize;
    let mut dim = 0usize;
    for card in lib.cards() {
        if script_js::is_catalog_dim(&card.name) {
            assert!(
                card.unloadable.is_some(),
                "{} dim must stay stamped",
                card.name
            );
            dim += 1;
            continue;
        }
        if card.source != ScriptSource::Catalog {
            continue;
        }
        match &card.unloadable {
            None => bright += 1,
            Some(u) => panic!("unexpected bright unloadable on {}: {u}", card.name),
        }
    }
    assert!(dim >= 1, "Woodcutter dim row");
    assert!(bright >= 3, "CookBot/HerbCard/TypeOnly stay bright");
    assert_eq!(
        lib.get(ScriptSource::Catalog, "HerbCard")
            .expect("HerbCard listed")
            .unloadable,
        None,
        "herbs.js remap must not stamp HerbCard unloadable"
    );
    assert_eq!(
        lib.get(ScriptSource::Catalog, "TypeOnly")
            .expect("TypeOnly listed")
            .unloadable,
        None,
        "type-only dangerZones must not block TypeOnly"
    );
}

#[test]
fn synthetic_bright_catalog_cards_start_without_not_impl() {
    if !crate::local_game_data::local_game_data_available(289) {
        return;
    }
    let selected = api::game_data::for_revision(api::selected::ClientRevision::R289)
        .expect("selected R289 game data for bright Start");
    let dir = scratch("start");
    let mut lib = register_fixture(&dir);
    let names: Vec<String> = lib
        .cards()
        .iter()
        .filter(|c| {
            c.source == ScriptSource::Catalog
                && !script_js::is_catalog_dim(&c.name)
                && c.unloadable.is_none()
        })
        .map(|c| c.name.clone())
        .collect();
    assert!(names.len() >= 3);

    let mut hits: BTreeSet<String> = BTreeSet::new();
    let cache = JsCache::new(dir.join("sib-cache"));
    let loading: Vec<StatInput<'static>> = common::FRESH_STATS
        .iter()
        .map(|row| StatInput {
            xp: 0,
            base: 0,
            effective: 0,
            ..*row
        })
        .collect();
    let mut snap = common::ingame_snapshot();
    snap.stats = &loading;
    snap.varps = &[];
    let loading_bytes = script_js::wire::encode_snapshot(&snap);
    snap.stats = &common::FRESH_STATS;
    let loaded_bytes = script_js::wire::encode_snapshot(&snap);

    for name in &names {
        if let Err(e) = lib.ensure_js(ScriptSource::Catalog, name) {
            hits.insert(format!("{name}: transpile {e}"));
            continue;
        }
        let Some(card) = lib.get(ScriptSource::Catalog, name).cloned() else {
            hits.insert(format!("{name}: missing after ensure_js"));
            continue;
        };
        let siblings = match script_js::resolve_sibling_modules(
            &card.path,
            &card.origin,
            &cache,
            CacheMeta {
                kind: ScriptKind::Compat,
                source: ScriptSource::Catalog,
                shape: Some(format!("{:?}", card.shape)),
                api_family: None,
            },
        ) {
            Ok(s) => s,
            Err(e) => {
                hits.insert(format!("{name}: siblings {e}"));
                continue;
            }
        };
        let bag = script::merge_bag(&card.settings_schema, &serde_json::Map::new(), None);
        for (scene, bytes) in [("pre-stats", &loading_bytes), ("loaded", &loaded_bytes)] {
            let iso = match LoadIsolate::spawn_with_content(
                card.js.clone(),
                card.shape,
                siblings.clone(),
                Some(Arc::clone(&selected)),
                Arc::new(api::named_banks::NamedBankFacts::empty()),
            ) {
                Ok(iso) => iso,
                Err(e) => {
                    hits.insert(format!("{name}: load {e}"));
                    break;
                }
            };
            if !bag.is_empty() {
                iso.post_settings_bag(&bag);
            }
            iso.post_snapshot(bytes.clone());
            iso.on_game_tick(1);
            let _ = iso.probe("true");
            for line in iso.drain_logs() {
                if throw_shaped(&line) {
                    hits.insert(format!("{name} ({scene}): {line}"));
                }
            }
            iso.join();
        }
    }
    assert!(
        hits.is_empty(),
        "fixture bright Start threw:\n{}",
        hits.iter().cloned().collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn synthetic_bright_catalog_module_graph_loads_at_probe_barrier() {
    let dir = scratch("graph");
    let root = dir.join("rs2b0t");
    write_fixture_catalog(&root);
    let mut lib = JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache"));
    lib.register_rs2b0t(&root, &dir.join("rs2b0t-path"))
        .expect("register");
    for name in ["CookBot", "HerbCard"] {
        lib.ensure_js(ScriptSource::Catalog, name)
            .unwrap_or_else(|e| panic!("{name} transpile: {e}"));
        let card = lib.get(ScriptSource::Catalog, name).cloned().unwrap();
        assert!(card.unloadable.is_none());
        let siblings = script_js::resolve_sibling_modules(
            &card.path,
            &card.origin,
            lib.cache(),
            CacheMeta {
                kind: ScriptKind::Compat,
                source: ScriptSource::Catalog,
                shape: Some(format!("{:?}", card.shape)),
                api_family: None,
            },
        )
        .expect("siblings");
        let iso = LoadIsolate::spawn_with_content(
            card.js.clone(),
            card.shape,
            siblings,
            None,
            Arc::new(api::named_banks::NamedBankFacts::empty()),
        )
        .expect("spawn");
        iso.probe("true").expect("probe barrier after spawn");
        assert_eq!(iso.poll_ready(), script_js::Ready::Ready);
        iso.join();
    }
}

#[test]
#[ignore = "requires the frozen ae56ccc95 catalog at RS2B0T"]
fn frozen_bright_catalog_cards_load_and_run_first_tick() {
    const FROZEN_REVISION: &str = "ae56ccc95ba5681b12f1f66ec2adea14427fe497";
    let Some(root) = api::test_support::local_asset_path("RS2B0T", "", "frozen ae56ccc95 catalog")
    else {
        return;
    };
    let revision = std::process::Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("run git rev-parse for the frozen catalog");
    assert!(
        revision.status.success(),
        "RS2B0T is not a git checkout: {}",
        String::from_utf8_lossy(&revision.stderr)
    );
    assert_eq!(
        String::from_utf8(revision.stdout).unwrap().trim(),
        FROZEN_REVISION,
        "RS2B0T must be the exact frozen catalog pin"
    );

    let dir = scratch("frozen-first-tick");
    let mut lib = JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache"));
    lib.register_rs2b0t(&root, &dir.join("rs2b0t-path"))
        .expect("register frozen catalog");

    let mut bright_names = Vec::new();
    let mut dim = 0usize;
    let mut preexisting_unresolved = 0usize;
    let mut failures = BTreeSet::new();
    for card in lib.cards() {
        if card.source != ScriptSource::Catalog {
            continue;
        }
        // Catalog dim policy also includes explicit special cases and
        // exact-version defects whose names are not in CATALOG_DIM.
        let is_dim = script_js::is_catalog_dim(&card.name)
            || card
                .unloadable
                .as_deref()
                .is_some_and(|reason| reason.starts_with("dim: "));
        if is_dim {
            if card.unloadable.is_none() {
                failures.insert(format!("{}: dim card has no unloadable reason", card.name));
            }
            dim += 1;
        } else if let Some(reason) = &card.unloadable {
            // The frozen baseline also rejects this exact import: there is no
            // served journal.js shim, while the pinned source contains journal.ts.
            if card.name == "AccountLeveler"
                && reason == "../../api/ai/quests/defs/dragonslayer/journal.js"
            {
                preexisting_unresolved += 1;
            } else {
                failures.insert(format!(
                    "{}: unexpected unloadable reason {reason}",
                    card.name
                ));
            }
        } else {
            bright_names.push(card.name.clone());
        }
    }
    assert!(
        !bright_names.is_empty(),
        "frozen catalog has no bright cards"
    );

    let cache = JsCache::new(dir.join("siblings"));
    let bytes = script_js::wire::encode_snapshot(&common::ingame_snapshot());
    let content = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
    let mut started = 0usize;
    for name in bright_names {
        if let Err(error) = lib.ensure_js(ScriptSource::Catalog, &name) {
            failures.insert(format!("{name}: transpile {error}"));
            continue;
        }
        let Some(card) = lib.get(ScriptSource::Catalog, &name).cloned() else {
            failures.insert(format!("{name}: missing after ensure_js"));
            continue;
        };
        let siblings = match script_js::resolve_sibling_modules(
            &card.path,
            &card.origin,
            &cache,
            CacheMeta {
                kind: ScriptKind::Compat,
                source: ScriptSource::Catalog,
                shape: Some(format!("{:?}", card.shape)),
                api_family: None,
            },
        ) {
            Ok(siblings) => siblings,
            Err(error) => {
                failures.insert(format!("{name}: siblings {error}"));
                continue;
            }
        };
        let mut settings_input = serde_json::Map::new();
        if name == "MuleCrafter" {
            // The default Crafter has bankFill=false and needs an Air talisman.
            // The shared first-tick snapshot has an empty inventory.
            // The outside Mule setup needs neither a talisman nor bank state.
            settings_input.insert("mode".into(), serde_json::Value::String("Mule".into()));
            settings_input.insert(
                "partner".into(),
                serde_json::Value::String("fixture".into()),
            );
            settings_input.insert(
                "meetingPoint".into(),
                serde_json::Value::String("Ruins (outside)".into()),
            );
        }
        let settings = script::merge_bag(&card.settings_schema, &settings_input, None);
        let isolate = match LoadIsolate::spawn_with_game_data(
            card.js.clone(),
            card.shape,
            siblings,
            content.clone(),
        ) {
            Ok(isolate) => isolate,
            Err(error) => {
                failures.insert(format!("{name}: load {error}"));
                continue;
            }
        };
        if !settings.is_empty() {
            isolate.post_settings_bag(&settings);
        }
        isolate.post_snapshot(bytes.clone());
        isolate.on_game_tick(1);
        if let Err(probe_error) = isolate.probe("true") {
            match isolate.poll_ready() {
                script_js::Ready::Failed(load_error)
                    if name == "MuleCrafter"
                        && load_error.contains("bankTile: unknown bank 'Falador East'") =>
                {
                    eprintln!("Frozen catalog expected baseline failure {name}: {load_error}");
                }
                script_js::Ready::Failed(load_error) => {
                    failures.insert(format!(
                        "{name}: loader failed after first-tick probe ({probe_error}): {load_error}"
                    ));
                }
                state => {
                    failures.insert(format!(
                        "{name}: first-tick probe {probe_error}; loader state {state:?}"
                    ));
                }
            }
        }
        for line in isolate.drain_logs() {
            if throw_shaped(&line) {
                failures.insert(format!("{name}: {line}"));
            }
        }
        isolate.join();
        started += 1;
    }
    eprintln!(
        "Frozen catalog first tick: {started} bright cards; {dim} dim cards skipped; {preexisting_unresolved} pre-existing unresolved imports skipped"
    );
    assert!(
        failures.is_empty(),
        "frozen catalog first-tick failures:\n{}",
        failures.into_iter().collect::<Vec<_>>().join("\n")
    );
}
