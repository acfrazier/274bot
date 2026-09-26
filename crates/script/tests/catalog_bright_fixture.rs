//! Synthetic mini-catalog regressions: bright Start, module-graph load, and
//! import-remap stamping (no `$RS2B0T` / persisted rs2b0t-path).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use script::isolate_fb::StatInput;
use script::load::{JsLibrary, LoadIsolate};
use script::{CacheMeta, JsCache, ScriptKind, ScriptSource};

mod common;

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
        if script::is_catalog_dim(&card.name) {
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
    let dir = scratch("start");
    let mut lib = register_fixture(&dir);
    let names: Vec<String> = lib
        .cards()
        .iter()
        .filter(|c| {
            c.source == ScriptSource::Catalog
                && !script::is_catalog_dim(&c.name)
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
    let loading_bytes = script::isolate_fb::encode_snapshot(&snap);
    snap.stats = &common::FRESH_STATS;
    let loaded_bytes = script::isolate_fb::encode_snapshot(&snap);
    let content = api::game_data::for_revision(client::io::ClientRevision::R274).unwrap();

    for name in &names {
        if let Err(e) = lib.ensure_js(ScriptSource::Catalog, name) {
            hits.insert(format!("{name}: transpile {e}"));
            continue;
        }
        let Some(card) = lib.get(ScriptSource::Catalog, name).cloned() else {
            hits.insert(format!("{name}: missing after ensure_js"));
            continue;
        };
        let siblings = match script::resolve_sibling_modules(
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
            let iso = match LoadIsolate::spawn_with_game_data(
                card.js.clone(),
                card.shape,
                siblings.clone(),
                content.clone(),
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
    let content = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
    for name in ["CookBot", "HerbCard"] {
        lib.ensure_js(ScriptSource::Catalog, name)
            .unwrap_or_else(|e| panic!("{name} transpile: {e}"));
        let card = lib.get(ScriptSource::Catalog, name).cloned().unwrap();
        assert!(card.unloadable.is_none());
        let siblings = script::resolve_sibling_modules(
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
        let iso = LoadIsolate::spawn_with_game_data(
            card.js.clone(),
            card.shape,
            siblings,
            content.clone(),
        )
        .expect("spawn");
        iso.probe("true").expect("probe barrier after spawn");
        assert_eq!(iso.poll_ready(), script::Ready::Ready);
        iso.join();
    }
}
