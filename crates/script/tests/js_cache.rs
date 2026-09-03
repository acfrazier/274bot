//! Task 2: SHA content-addressed JS cache + hot-reload (never write `$RS2B0T`).

use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use script::load::JsLibrary;
use script::{CacheMeta, JsCache, ScriptKind, ScriptSource};

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
        .get_or_transpile(
            &origin,
            CLASS_TS.as_bytes(),
            CacheMeta {
                kind: ScriptKind::Compat,
                source: ScriptSource::File,
                shape: Some("CompatClass".into()),
            },
        )
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

    let meta = CacheMeta {
        kind: ScriptKind::Compat,
        source: ScriptSource::File,
        shape: None,
    };
    let first = cache
        .get_or_transpile(&origin, CLASS_TS.as_bytes(), meta.clone())
        .unwrap();
    let mtime = object_mtime(&cache, &first.sha256);
    thread::sleep(Duration::from_millis(20));

    let second = cache
        .get_or_transpile(&origin, CLASS_TS.as_bytes(), meta)
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
        .get_or_transpile(
            &path,
            NATIVE_TICK.as_bytes(),
            CacheMeta {
                kind: ScriptKind::NativeTick,
                source: ScriptSource::File,
                shape: Some("NativeTick".into()),
            },
        )
        .expect("js origin caches verbatim");
    assert_eq!(cached.js, NATIVE_TICK);
}

#[cfg(unix)]
#[test]
fn cache_dirs_are_private_on_unix() {
    use std::os::unix::fs::PermissionsExt;

    let dir = scratch("dir_mode");
    let cache_root = dir.join("js-cache");
    let cache = JsCache::new(cache_root.clone());
    let origin = write_file(&dir, "bot.ts", CLASS_TS);
    cache
        .get_or_transpile(
            &origin,
            CLASS_TS.as_bytes(),
            CacheMeta {
                kind: ScriptKind::Compat,
                source: ScriptSource::File,
                shape: None,
            },
        )
        .expect("cache write creates layout");

    let root_mode = std::fs::metadata(&cache_root)
        .expect("cache root")
        .permissions()
        .mode()
        & 0o777;
    let objects_mode = std::fs::metadata(cache_root.join("objects"))
        .expect("objects dir")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(root_mode, 0o700, "js-cache root must be owner-only");
    assert_eq!(objects_mode, 0o700, "objects/ must be owner-only");
}

#[test]
fn manifest_records_script_kind_source_and_media() {
    let dir = scratch("manifest_meta");
    let cache_root = dir.join("js-cache");
    let cache = JsCache::new(cache_root.clone());
    let origin = write_file(&dir, "tickbot.js", NATIVE_TICK);
    cache
        .get_or_transpile(
            &origin,
            NATIVE_TICK.as_bytes(),
            CacheMeta {
                kind: ScriptKind::NativeTick,
                source: ScriptSource::File,
                shape: Some("NativeTick".into()),
            },
        )
        .expect("cache miss writes manifest");

    let raw = std::fs::read_to_string(cache_root.join("manifest.json")).expect("manifest");
    let manifest: serde_json::Value = serde_json::from_str(&raw).expect("manifest json");
    let entry = manifest["objects"]
        .as_object()
        .and_then(|o| o.values().next())
        .expect("one manifest entry");
    assert_eq!(entry["kind"], "NativeTick");
    assert_eq!(entry["source"], "File");
    assert_eq!(entry["media"], "js");
    assert_eq!(entry["shape"], "NativeTick");
    assert!(entry["origin"].as_str().unwrap().ends_with("tickbot.js"));
}

#[test]
fn persist_and_restore_keeps_catalog_vs_file_provenance() {
    let dir = scratch("persist_provenance");
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
    let mut lib = JsLibrary::with_cache(store.clone(), cache_root.clone());
    lib.load(&file_copy).expect("file BoneBurier loads");
    lib.register_rs2b0t(&root, &dir.join("rs2b0t-path"))
        .expect("catalog registers");

    let store_raw = std::fs::read_to_string(&store).expect("persisted store");
    let entries: Vec<serde_json::Value> =
        serde_json::from_str(&store_raw).expect("store is a json array");
    assert_eq!(
        entries.len(),
        1,
        "js-scripts.json must list File cards only, not catalog paths"
    );
    assert!(
        entries[0]["path"]
            .as_str()
            .unwrap()
            .contains("local/BoneBurier.ts"),
        "persisted path is the operator file copy"
    );

    let mut lib2 = JsLibrary::with_cache(store, cache_root);
    lib2.restore().expect("restore file cards");
    lib2.register_rs2b0t(&root, &dir.join("rs2b0t-path"))
        .expect("catalog re-register after restart");

    assert_eq!(lib2.cards().len(), 2);
    assert_eq!(
        lib2.get(ScriptSource::File, "BoneBurier")
            .expect("file card")
            .source,
        ScriptSource::File
    );
    assert_eq!(
        lib2.get(ScriptSource::Catalog, "BoneBurier")
            .expect("catalog card")
            .source,
        ScriptSource::Catalog
    );
}

fn write_two_card_catalog(dir: &std::path::Path) -> std::path::PathBuf {
    let root = dir.join("rs2b0t");
    let scripts = root.join("src/bot/scripts");
    std::fs::create_dir_all(scripts.join("BoneBurier")).unwrap();
    std::fs::create_dir_all(scripts.join("ShopRunner")).unwrap();
    std::fs::write(
        scripts.join("index.ts"),
        r#"
import BoneBurier from './BoneBurier/BoneBurier.js';
import ShopRunner from './ShopRunner/ShopRunner.js';
ScriptRegistry.register({ name: 'BoneBurier', create: () => new BoneBurier() });
ScriptRegistry.register({ name: 'ShopRunner', create: () => new ShopRunner() });
"#,
    )
    .unwrap();
    std::fs::write(
        scripts.join("BoneBurier/BoneBurier.ts"),
        "export default class BoneBurier extends LoopingBot { override loop() {} }",
    )
    .unwrap();
    std::fs::write(
        scripts.join("ShopRunner/ShopRunner.ts"),
        "export default class ShopRunner extends LoopingBot { override loop() {} }",
    )
    .unwrap();
    root
}

#[test]
fn catalog_register_does_not_transpile_until_ensure_js() {
    let dir = scratch("catalog_lazy");
    let root = write_two_card_catalog(&dir);
    let cache_root = dir.join("js-cache");
    let mut lib = JsLibrary::with_cache(dir.join("js-scripts.json"), cache_root);
    lib.register_rs2b0t(&root, &dir.join("rs2b0t-path"))
        .expect("catalog fill");

    let bone = lib
        .get(ScriptSource::Catalog, "BoneBurier")
        .expect("BoneBurier card");
    assert!(
        bone.js.is_empty(),
        "catalog fill is origin/classify only — JS must not run until the operator asks"
    );
    assert!(
        !bone.sha256.is_empty(),
        "origin SHA is known without transpile"
    );
    assert!(
        !lib.cache().object_path(&bone.sha256).is_file(),
        "no cache object until ensure_js"
    );
    assert_eq!(lib.cards_needing_transpile().len(), 2);

    lib.ensure_js(ScriptSource::Catalog, "BoneBurier")
        .expect("warm BoneBurier");
    let bone = lib
        .get(ScriptSource::Catalog, "BoneBurier")
        .expect("BoneBurier after warm");
    assert!(
        !bone.js.is_empty(),
        "ensure_js writes cached JS for that card"
    );
    assert!(lib.cache().object_path(&bone.sha256).is_file());

    let shop = lib
        .get(ScriptSource::Catalog, "ShopRunner")
        .expect("ShopRunner still lazy");
    assert!(
        shop.js.is_empty(),
        "warming one card must not transpile the rest of the catalog"
    );
    assert!(!lib.cache().object_path(&shop.sha256).is_file());
    assert_eq!(lib.cards_needing_transpile().len(), 1);
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
