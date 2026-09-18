//! Canonical catalog cards: herb imports evaluated from each card's script folder
//! (same specifiers and transitive siblings as AutoFighter / HerbCleaner).

use std::path::PathBuf;

use script::load::{JsLibrary, LoadIsolate, LoadShape};
use script::{CacheMeta, JsCache, ScriptKind, ScriptSource};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "274bot-herb-data-{}-{}",
        std::process::id(),
        name
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn card_origin_path(name: &str) -> (std::path::PathBuf, String) {
    let root = script::rs2b0t_root().expect("$RS2B0T canonical reference");
    let dir = scratch(name);
    let mut lib = JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache"));
    lib.register_rs2b0t(&root, &dir.join("rs2b0t-path"))
        .expect("catalog register");
    let card = lib
        .get(ScriptSource::Catalog, name)
        .cloned()
        .unwrap_or_else(|| panic!("{name} listed"));
    assert_eq!(
        card.unloadable, None,
        "{name} must not be import-stamped unloadable"
    );
    (card.path, card.origin)
}

fn spawn_from_card_path(
    card_path: &std::path::Path,
    card_origin: &str,
    eval_src: &str,
) -> LoadIsolate {
    let siblings = script::resolve_sibling_modules(
        card_path,
        card_origin,
        &JsCache::new(
            card_path
                .parent()
                .unwrap()
                .join(format!("sib-cache-{}", std::process::id())),
        ),
        CacheMeta {
            kind: ScriptKind::Compat,
            source: ScriptSource::Catalog,
            shape: Some("CompatClass".into()),
            api_family: None,
        },
    )
    .expect("canonical siblings resolve");
    let data = api::game_data::for_revision(client::io::ClientRevision::R274).unwrap();
    LoadIsolate::spawn_with_game_data(
        eval_src.to_string(),
        LoadShape::CompatClass,
        siblings,
        data,
    )
    .expect("spawn eval isolate")
}

#[test]
fn autofighter_folder_evaluates_herbs_import_with_selected_facts() {
    let Some(_) = script::rs2b0t_root() else {
        return;
    };
    let (path, _origin) = card_origin_path("AutoFighter");
    let src = r#"
import { HERBS, HERB_OPTIONS } from '../../data/herbs.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__herbProbe = {
            count: HERBS.length,
            guam: HERBS.find(h => h.key === 'guam') || null,
            options: HERB_OPTIONS.length,
            hasGuamOption: HERB_OPTIONS.includes('Guam leaf'),
            marrentillLevel: HERBS.find(h => h.key === 'marrentill')?.level ?? null,
        };
    }
}
"#;
    let iso = spawn_from_card_path(&path, src, src);
    iso.on_game_tick(1);
    let probe = iso.probe("__herbProbe").unwrap();
    assert!(probe["count"].as_u64().unwrap() >= 14);
    assert_eq!(probe["guam"]["id"], 249);
    assert_eq!(probe["guam"]["unidId"], 199);
    assert_eq!(probe["guam"]["level"], 3);
    assert_eq!(probe["marrentillLevel"], 5);
    assert!(probe["hasGuamOption"].as_bool().unwrap());
    iso.join();
}

#[test]
fn herb_cleaner_folder_evaluates_transitive_logic_and_herbs() {
    let Some(_) = script::rs2b0t_root() else {
        return;
    };
    let (path, _origin) = card_origin_path("HerbCleaner");
    let src = r#"
import { HERBS, HERB_OPTIONS } from '../../data/herbs.js';
import { eligibleHerbs, herbByUnidId } from './HerbCleanerLogic.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__herbProbe = {
            count: HERBS.length,
            options: HERB_OPTIONS.length,
            guamUnid: herbByUnidId(199)?.key ?? null,
            eligible99: eligibleHerbs(99, []).length,
        };
    }
}
"#;
    let iso = spawn_from_card_path(&path, src, src);
    iso.on_game_tick(1);
    let probe = iso.probe("__herbProbe").unwrap();
    assert!(probe["count"].as_u64().unwrap() >= 14);
    assert_eq!(probe["guamUnid"], "guam");
    assert!(probe["eligible99"].as_u64().unwrap() >= 14);
    assert!(probe["options"].as_u64().unwrap() >= 14);
    iso.join();
}
