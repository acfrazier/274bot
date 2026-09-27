//! Selected herb facts reach compat cards through the remapped `herbs.js`
//! surface and same-folder sibling graphs (no optional catalog checkout).

use std::path::PathBuf;

use script::load::{LoadIsolate, LoadShape};
use script::{CacheMeta, JsCache, ScriptKind, ScriptSource};

fn scratch(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("274bot-herb-data-{}-{}", std::process::id(), name));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn data() -> std::sync::Arc<api::game_data::SelectedGameData> {
    api::game_data::for_revision(client::io::ClientRevision::R274).unwrap()
}

fn spawn_from_card_folder(
    card_dir: &std::path::Path,
    card_origin: &str,
    eval_src: &str,
) -> LoadIsolate {
    let siblings = script::resolve_sibling_modules(
        &card_dir.join("Card.ts"),
        card_origin,
        &JsCache::new(card_dir.join(format!("sib-cache-{}", std::process::id()))),
        CacheMeta {
            kind: ScriptKind::Compat,
            source: ScriptSource::File,
            shape: Some("CompatClass".into()),
            api_family: None,
        },
    )
    .expect("siblings resolve");
    LoadIsolate::spawn_with_game_data(
        eval_src.to_string(),
        LoadShape::CompatClass,
        siblings,
        data(),
    )
    .expect("spawn eval isolate")
}

#[test]
fn card_folder_evaluates_herbs_import_with_selected_facts() {
    let dir = scratch("autofighter-shaped");
    let card_dir = dir.join("AutoFighter");
    std::fs::create_dir_all(&card_dir).unwrap();
    let origin = r#"
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
    std::fs::write(card_dir.join("Card.ts"), origin).unwrap();
    let iso = spawn_from_card_folder(&card_dir, origin, origin);
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
fn logic_sibling_evaluates_transitive_herbs_with_selected_facts() {
    let dir = scratch("herb-cleaner-shaped");
    let card_dir = dir.join("HerbCleaner");
    std::fs::create_dir_all(&card_dir).unwrap();
    std::fs::write(
        card_dir.join("HerbCleanerLogic.ts"),
        r#"
import { HERBS, HERB_OPTIONS } from '../../data/herbs.js';
export function herbByUnidId(unid) {
    return HERBS.find(h => h.unidId === unid) ?? null;
}
export function eligibleHerbs(level, skip) {
    const blocked = new Set(skip || []);
    return HERBS.filter(h => h.level <= level && !blocked.has(h.key));
}
export { HERBS, HERB_OPTIONS };
"#,
    )
    .unwrap();
    let origin = r#"
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
    std::fs::write(card_dir.join("Card.ts"), origin).unwrap();
    let iso = spawn_from_card_folder(&card_dir, origin, origin);
    iso.on_game_tick(1);
    let probe = iso.probe("__herbProbe").unwrap();
    assert!(probe["count"].as_u64().unwrap() >= 14);
    assert_eq!(probe["guamUnid"], "guam");
    assert!(probe["eligible99"].as_u64().unwrap() >= 14);
    assert!(probe["options"].as_u64().unwrap() >= 14);
    iso.join();
}
