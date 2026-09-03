//! Task 7: same-folder sibling modules (./Logic.js → .ts twin → cache).

use std::path::PathBuf;

use script::load::{resolve_sibling_modules, scan_same_folder_js_imports, sibling_module_url};
use script::{CacheMeta, JsCache, JsLibrary, ScriptKind, ScriptSource};

fn temp_dir(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("274bot-load-sibling-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const MAIN_WITH_SIBLING: &str = r#"
import logic from './Logic.js';
export default class Main extends LoopingBot {
    override loop() { logic.run(); }
}
"#;

const LOGIC_TS: &str = r#"
export default { run() { globalThis.__logic_ran = 1; } };
"#;

#[test]
fn scan_same_folder_js_imports_finds_quoted_relative_js() {
    let imports = scan_same_folder_js_imports(MAIN_WITH_SIBLING);
    assert_eq!(imports, vec!["./Logic.js"]);
}

#[test]
fn sibling_module_url_maps_to_bot_module_dir() {
    assert_eq!(
        sibling_module_url("./Logic.js"),
        Some("/rs2b0t/bot/scripts/bot/Logic.js".into())
    );
    assert!(sibling_module_url("../evil.js").is_none());
}

#[test]
fn sibling_logic_ts_is_cached_and_registered_without_v8() {
    let dir = temp_dir("logic");
    let cache_root = dir.join("js-cache");
    let cache = JsCache::new(cache_root);
    let card_dir = dir.join("bots");
    std::fs::create_dir_all(&card_dir).unwrap();
    let main_path = card_dir.join("Main.ts");
    std::fs::write(&main_path, MAIN_WITH_SIBLING).unwrap();
    std::fs::write(card_dir.join("Logic.ts"), LOGIC_TS).unwrap();

    let siblings = resolve_sibling_modules(
        &main_path,
        MAIN_WITH_SIBLING,
        &cache,
        CacheMeta {
            kind: ScriptKind::Compat,
            source: ScriptSource::File,
            shape: Some("CompatClass".into()),
        },
    )
    .expect("resolve siblings");

    assert_eq!(siblings.len(), 1);
    assert_eq!(siblings[0].0, "/rs2b0t/bot/scripts/bot/Logic.js");
    assert!(
        siblings[0].1.contains("run"),
        "transpiled sibling JS: {}",
        siblings[0].1
    );
    let logic_bytes = std::fs::read(card_dir.join("Logic.ts")).unwrap();
    let cached = cache
        .get_or_transpile(
            &card_dir.join("Logic.ts"),
            &logic_bytes,
            CacheMeta {
                kind: ScriptKind::Compat,
                source: ScriptSource::File,
                shape: None,
            },
        )
        .expect("Logic.ts cached");
    assert!(
        cache.object_path(&cached.sha256).is_file(),
        "Logic.ts bytes must land in js-cache"
    );
}

#[test]
fn load_under_fake_rs2b0t_root_is_file_not_catalog() {
    let dir = temp_dir("under-root");
    let root = dir.join("rs2b0t");
    let scripts = root.join("src/bot/scripts");
    std::fs::create_dir_all(scripts.join("External")).unwrap();
    std::fs::write(
        scripts.join("index.ts"),
        r#"
import X from './External/External.js';
ScriptRegistry.register({ name: 'CatalogOnly', create: () => new X() });
"#,
    )
    .unwrap();
    std::fs::write(
        scripts.join("External/External.ts"),
        "export default class External extends LoopingBot { override loop() {} }",
    )
    .unwrap();

    let external = scripts.join("External/External.ts");
    let store = dir.join("js-scripts.json");
    let cache_root = dir.join("js-cache");
    let mut lib = JsLibrary::with_cache(store, cache_root);

    let card = lib.load(&external).expect("Load under rs2b0t tree");
    assert_eq!(card.source, ScriptSource::File);
    assert_eq!(card.kind, ScriptKind::Compat);
    assert!(
        lib.get(ScriptSource::Catalog, "CatalogOnly").is_none(),
        "Load must not parse index.ts / register catalog cards"
    );
    assert_eq!(lib.cards().len(), 1);
}
