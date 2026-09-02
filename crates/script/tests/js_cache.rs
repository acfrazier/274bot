//! Task 2: SHA content-addressed JS cache + hot-reload (never write `$RS2B0T`).

use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use script::load::JsLibrary;
use script::{JsCache, ScriptKind, ScriptSource};

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("274bot-js-cache-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn scratch(name: &str) -> PathBuf {
    let dir = temp_dir().join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const NATIVE_TICK: &str =
    "export function tick(api) { globalThis.__rs_n = (globalThis.__rs_n || 0) + 1 }";

const CLASS_TS: &str = r#"
export default class Burier extends LoopingBot {
    override loopDelay = 600;
    private n: number = 0;
    override loop() { this.n += 1; }
}
"#;

fn write_file(dir: &PathBuf, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&path, body).unwrap();
    path
}

fn object_mtime(cache: &JsCache, sha: &str) -> std::time::SystemTime {
    std::fs::metadata(cache.object_path(sha))
        .expect("cached object exists")
        .modified()
        .expect("mtime")
}

#[test]
fn first_import_writes_cache_object() {
    let dir = scratch("first_import");
    let cache_root = dir.join("js-cache");
    let cache = JsCache::new(cache_root.clone());
    let origin = write_file(&dir, "bot.ts", CLASS_TS);

    let cached = cache
        .get_or_transpile(&origin, CLASS_TS.as_bytes())
        .expect("first import caches");
    assert!(!cached.sha256.is_empty());
    assert!(cache.object_path(&cached.sha256).is_file());
    assert!(
        cached.js.contains("extends LoopingBot"),
        "transpiled JS returned: {}",
        cached.js
    );
}

#[test]
fn second_same_bytes_hits_cache_without_rewriting_object() {
    let dir = scratch("cache_hit");
    let cache_root = dir.join("js-cache");
    let cache = JsCache::new(cache_root);
    let origin = write_file(&dir, "bot.ts", CLASS_TS);

    let first = cache
        .get_or_transpile(&origin, CLASS_TS.as_bytes())
        .unwrap();
    let mtime = object_mtime(&cache, &first.sha256);
    thread::sleep(Duration::from_millis(20));

    let second = cache
        .get_or_transpile(&origin, CLASS_TS.as_bytes())
        .unwrap();
    assert_eq!(first.sha256, second.sha256);
    assert_eq!(first.js, second.js);
    assert_eq!(
        object_mtime(&cache, &second.sha256),
        mtime,
        "cache hit must not rewrite the object file"
    );
}

#[test]
fn mutated_origin_produces_new_object_and_refresh_keeps_one_card() {
    let dir = scratch("hot_reload");
    let store = dir.join("js-scripts.json");
    let cache_root = dir.join("js-cache");
    let mut lib = JsLibrary::with_cache(store, cache_root);
    let path = write_file(&dir, "tickbot.ts", CLASS_TS);

    let card = lib.load(&path).expect("load ts bot");
    let old_sha = card.sha256.clone();
    assert_eq!(lib.cards().len(), 1);

    let mutated = CLASS_TS.replace("600", "601");
    std::fs::write(&path, &mutated).unwrap();

    lib.refresh(script::ScriptSource::File, "tickbot")
        .expect("refresh after edit");
    assert_eq!(lib.cards().len(), 1, "same (source, name) updates in place");
    let updated = lib
        .get(script::ScriptSource::File, "tickbot")
        .expect("card still present");
    assert_ne!(updated.sha256, old_sha, "SHA must change after origin edit");
    assert!(updated.js.contains("601"), "cached JS reflects mutation");
}

#[test]
fn catalog_and_file_bone_burier_are_two_cards() {
    let dir = scratch("two_bone_buriers");
    let root = dir.join("rs2b0t");
    let scripts = root.join("src/bot/scripts");
    std::fs::create_dir_all(scripts.join("BoneBurier")).unwrap();
    std::fs::write(
        scripts.join("index.ts"),
        r#"
import BoneBurier from './BoneBurier/BoneBurier.js';
ScriptRegistry.register({ name: 'BoneBurier', create: () => new BoneBurier() });
"#,
    )
    .unwrap();
    std::fs::write(
        scripts.join("BoneBurier/BoneBurier.ts"),
        "export default class BoneBurier extends LoopingBot { override loop() {} }",
    )
    .unwrap();

    let file_copy = write_file(
        &dir,
        "local/BoneBurier.ts",
        "export default class BoneBurier extends LoopingBot { override loop() { this.x = 1 } }",
    );

    let store = dir.join("js-scripts.json");
    let cache_root = dir.join("js-cache");
    let mut lib = JsLibrary::with_cache(store, cache_root);
    lib.load(&file_copy).expect("file BoneBurier loads");
    lib.register_rs2b0t(&root, &dir.join("rs2b0t-path"))
        .expect("catalog registers");

    let file_card = lib
        .get(ScriptSource::File, "BoneBurier")
        .expect("file card");
    let catalog_card = lib
        .get(ScriptSource::Catalog, "BoneBurier")
        .expect("catalog card");
    assert_ne!(file_card.sha256, catalog_card.sha256);
    assert_eq!(file_card.source, ScriptSource::File);
    assert_eq!(catalog_card.source, ScriptSource::Catalog);
    assert_eq!(lib.cards().len(), 2);
}

#[test]
fn rs2b0t_shaped_tree_gets_no_js_beside_ts() {
    let dir = scratch("no_sidecar_js");
    let root = dir.join("rs2b0t");
    let scripts = root.join("src/bot/scripts");
    std::fs::create_dir_all(scripts.join("BoneBurier")).unwrap();
    std::fs::write(
        scripts.join("index.ts"),
        r#"
import BoneBurier from './BoneBurier/BoneBurier.js';
ScriptRegistry.register({ name: 'BoneBurier', create: () => new BoneBurier() });
"#,
    )
    .unwrap();
    let ts_path = scripts.join("BoneBurier/BoneBurier.ts");
    std::fs::write(
        &ts_path,
        "export default class BoneBurier extends LoopingBot { override loop() {} }",
    )
    .unwrap();

    let store = dir.join("js-scripts.json");
    let cache_root = dir.join("js-cache");
    let mut lib = JsLibrary::with_cache(store, cache_root);
    lib.register_rs2b0t(&root, &dir.join("rs2b0t-path"))
        .expect("catalog fill uses cache only under js-cache root");

    let sidecar = ts_path.with_extension("js");
    assert!(
        !sidecar.exists(),
        "must never write .js beside origin .ts in $RS2B0T tree: {}",
        sidecar.display()
    );

    // Plain JS origin is hashed in cache, not copied beside the file.
    let js_path = write_file(&dir, "tickbot.js", NATIVE_TICK);
    lib.load(&js_path).expect("load plain js");
    assert!(
        !dir.join("tickbot.transpiled.js").exists(),
        "no ad-hoc js emitted next to operator files"
    );
}

#[test]
fn js_origin_is_hashed_not_transpiled() {
    let dir = scratch("js_origin");
    let cache = JsCache::new(dir.join("js-cache"));
    let path = write_file(&dir, "bot.js", NATIVE_TICK);
    let cached = cache
        .get_or_transpile(&path, NATIVE_TICK.as_bytes())
        .expect("js origin caches verbatim");
    assert_eq!(cached.js, NATIVE_TICK);
}

#[test]
fn load_populates_kind_source_and_sha_on_card() {
    let dir = scratch("card_fields");
    let mut lib = JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache"));
    let path = write_file(&dir, "tickbot.js", NATIVE_TICK);
    let card = lib.load(&path).expect("load");
    assert_eq!(card.kind, ScriptKind::NativeTick);
    assert_eq!(card.source, ScriptSource::File);
    assert!(!card.sha256.is_empty());
    assert_eq!(card.js, NATIVE_TICK);
    assert_eq!(card.origin, NATIVE_TICK);
}
