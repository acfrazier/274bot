//! Retained per-source load/transpile/import/runtime-load failures.
//!
//! Display name is never identity. One success must not clear another
//! card's error. A failed replacement keeps the working registration.

use std::path::{Path, PathBuf};

use script::load::{JsLibrary, LoadStage};
use script::{ApiFamily, ScriptSource};

const GOOD: &str = "export default class T extends LoopingBot { override loop() {} }\n";
const GOOD_V2: &str = "export const apiVersion = 2;\nexport function tick(api) { api._n = 1; }\n";
const BAD_PARSE: &str =
    "export default class T extends LoopingBot { override loop() { const x = \"unterminated } }\n";
const BAD_IMPORT: &str = concat!(
    "import x from '../../event/webwalk/Something.js';\n",
    "export default class T extends LoopingBot { override loop() {} }\n"
);
const BAD_RUNTIME: &str = concat!(
    "throw new Error('runtime boom');\n",
    "export default class T extends LoopingBot { override loop() {} }\n"
);
const V1_ON_NATIVE: &str =
    "export const apiVersion = 1;\nexport function tick(api) { api._n = 1; }\n";
const V2_ON_COMPAT: &str = concat!(
    "export const apiVersion = 2;\n",
    "export default class T extends LoopingBot { override loop() {} }\n"
);

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "274bot-load-diagnostics-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn library(dir: &Path) -> JsLibrary {
    JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache"))
}

fn write(dir: &Path, rel: &str, src: &str) -> PathBuf {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&path, src).unwrap();
    path
}

fn fake_catalog(root: &Path, bots: &[(&str, &str)]) {
    let scripts = root.join("src/bot/scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    let mut index =
        String::from("import { ScriptRegistry } from '../runtime/ScriptRegistry.js';\n");
    for (name, src) in bots {
        let folder = name.replace(' ', "");
        index.push_str(&format!("import {folder} from './{folder}/{folder}.js';\n"));
        let d = scripts.join(&folder);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(format!("{folder}.ts")), src).unwrap();
    }
    for (name, _) in bots {
        let folder = name.replace(' ', "");
        index.push_str(&format!(
            "ScriptRegistry.register({{ name: '{name}', description: '{name}', category: 'Test', create: () => new {folder}() }});\n"
        ));
    }
    std::fs::write(scripts.join("index.ts"), index).unwrap();
}

#[test]
fn mixed_file_batch_keeps_both_failures_after_success() {
    let dir = scratch("mixed-file");
    let mut js = library(&dir);
    let a = write(&dir, "a/bot.ts", BAD_PARSE);
    let b = write(&dir, "b/bot.ts", BAD_PARSE);
    let c = write(&dir, "ok.ts", GOOD);

    assert!(js.load(&a).is_err());
    assert!(js.load(&b).is_err());
    assert!(js.load(&c).is_ok());

    let failures = js.load_failures();
    assert_eq!(failures.len(), 2, "{failures:?}");
    assert!(failures.iter().any(|f| f.path == a && f.name == "bot"));
    assert!(failures.iter().any(|f| f.path == b && f.name == "bot"));
    assert!(!failures.iter().any(|f| f.name == "ok"));
    assert!(failures
        .iter()
        .all(|f| f.stage == LoadStage::ParseTranspile));
    assert!(failures
        .iter()
        .all(|f| f.line.is_some() || f.diagnostic.contains("ts parse")));
    let named = js.named_failure_output();
    assert!(named.contains("bot") && named.contains(&a.display().to_string()));
    assert!(named.contains(&b.display().to_string()));
}

#[test]
fn duplicate_display_names_are_keyed_by_path() {
    let dir = scratch("dup-name");
    let mut js = library(&dir);
    let one = write(&dir, "one/bot.ts", BAD_PARSE);
    let two = write(&dir, "two/bot.ts", BAD_IMPORT);

    assert!(js.load(&one).is_err());
    let imported = js.load(&two).expect("unloadable file still registers");
    assert_eq!(
        imported.unloadable.as_deref(),
        Some("../../event/webwalk/Something.js")
    );

    let failures = js.load_failures();
    assert_eq!(failures.len(), 2, "{failures:?}");
    assert_ne!(failures[0].identity_key, failures[1].identity_key);
    assert!(failures
        .iter()
        .any(|f| { f.path == one && f.stage == LoadStage::ParseTranspile && f.name == "bot" }));
    assert!(failures.iter().any(|f| {
        f.path == two
            && f.stage == LoadStage::ImportResolution
            && f.sibling.as_deref() == Some("../../event/webwalk/Something.js")
    }));
}

#[test]
fn corrected_retry_clears_only_the_affected_identity() {
    let dir = scratch("retry");
    let mut js = library(&dir);
    let keep = write(&dir, "keep.ts", BAD_PARSE);
    let fix = write(&dir, "fix.ts", BAD_PARSE);
    assert!(js.load(&keep).is_err());
    assert!(js.load(&fix).is_err());
    assert_eq!(js.load_failures().len(), 2);

    std::fs::write(&fix, GOOD).unwrap();
    assert!(js.load(&fix).is_ok());

    let failures = js.load_failures();
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert_eq!(failures[0].path, keep);
    assert_eq!(failures[0].name, "keep");
}

#[test]
fn failed_replace_preserves_working_card_and_records_attempt() {
    let dir = scratch("replace");
    let mut js = library(&dir);
    let path = write(&dir, "live.ts", GOOD);
    let card = js.load(&path).unwrap();
    let old_js = card.js.clone();
    let old_fp = js
        .stored_fingerprint(&card.identity_key())
        .unwrap()
        .to_string();
    assert!(js.load_failures().is_empty());

    std::fs::write(&path, BAD_RUNTIME).unwrap();
    let prepared = js
        .prepare_card_unvalidated(ScriptSource::File, &path.to_string_lossy())
        .unwrap();
    let err = prepared.validate().unwrap_err();
    js.record_prepared_failure(&prepared, &err);
    assert!(err.contains("runtime boom"), "{err}");

    let live = js.get(ScriptSource::File, &path.to_string_lossy()).unwrap();
    assert_eq!(live.js, old_js);
    assert_eq!(live.origin, GOOD);

    let failures = js.load_failures();
    assert_eq!(failures.len(), 1);
    let f = &failures[0];
    assert_eq!(f.identity_key, card.identity_key());
    assert_eq!(f.stage, LoadStage::RuntimeLoad);
    assert_ne!(f.fingerprint, old_fp);
    assert!(!f.fingerprint.is_empty());
}

#[test]
fn catalog_mixed_batch_retains_named_failures() {
    let dir = scratch("catalog-mixed");
    let root = dir.join("catalog");
    fake_catalog(
        &root,
        &[("GoodBot", GOOD), ("BadParse", GOOD), ("BadImport", GOOD)],
    );
    let mut js = library(&dir);
    js.register_rs2b0t(&root, &dir.join("rs2b0t-path")).unwrap();
    js.ensure_js(ScriptSource::Catalog, "GoodBot").unwrap();
    js.ensure_js(ScriptSource::Catalog, "BadParse").unwrap();
    js.ensure_js(ScriptSource::Catalog, "BadImport").unwrap();
    let good_js = js.get(ScriptSource::Catalog, "GoodBot").unwrap().js.clone();
    let bad_parse_js = js
        .get(ScriptSource::Catalog, "BadParse")
        .unwrap()
        .js
        .clone();
    let bad_import_origin = js
        .get(ScriptSource::Catalog, "BadImport")
        .unwrap()
        .origin
        .clone();

    std::fs::write(
        root.join("src/bot/scripts/GoodBot/GoodBot.ts"),
        "export default class T extends LoopingBot { override loop() { return; } }\n",
    )
    .unwrap();
    std::fs::write(root.join("src/bot/scripts/BadParse/BadParse.ts"), BAD_PARSE).unwrap();
    std::fs::write(
        root.join("src/bot/scripts/BadImport/BadImport.ts"),
        BAD_IMPORT,
    )
    .unwrap();

    let diff = js.diff_catalog(&root).unwrap();
    assert!(diff.changed.iter().any(|n| n == "GoodBot"));
    let mut prepared_ok = Vec::new();
    for name in &diff.changed.clone() {
        if let Ok(prepared) = js.prepare_card_unvalidated(ScriptSource::Catalog, name) {
            if let Err(error) = prepared.validate() {
                js.record_prepared_failure(&prepared, &error);
            } else {
                prepared_ok.push(prepared);
            }
        }
    }
    assert_eq!(prepared_ok.len(), 1);
    assert_eq!(prepared_ok[0].card.name, "GoodBot");
    for item in prepared_ok {
        js.commit_prepared(item).unwrap();
    }
    let report = js.apply_catalog_diff(diff);
    let good = js.get(ScriptSource::Catalog, "GoodBot").unwrap();
    assert_ne!(good.js, good_js);
    let bad_parse = js.get(ScriptSource::Catalog, "BadParse").unwrap();
    assert_eq!(bad_parse.js, bad_parse_js);
    let bad_import = js.get(ScriptSource::Catalog, "BadImport").unwrap();
    assert_eq!(bad_import.origin, bad_import_origin);
    assert!(
        js.load_failures().len() >= 2,
        "both catalog failures stay: {:?}",
        js.load_failures()
    );
    assert!(js
        .load_failures()
        .iter()
        .any(|f| f.name == "BadParse" && f.stage == LoadStage::ParseTranspile));
    assert!(js.load_failures().iter().any(|f| f.name == "BadImport"
        && (f.stage == LoadStage::ImportResolution || f.diagnostic.contains("unloadable"))));
    assert!(!js.load_failures().iter().any(|f| f.name == "GoodBot"));
    let named = report.named_failure_output();
    assert!(
        named.contains("BadParse") || js.named_failure_output().contains("BadParse"),
        "{named} / {}",
        js.named_failure_output()
    );
}

#[test]
fn v1_and_v2_family_errors_keep_stable_identities() {
    let dir = scratch("family");
    let mut js = library(&dir);
    let v1 = write(&dir, "native-v1.ts", V1_ON_NATIVE);
    let v2 = write(&dir, "compat-v2.ts", V2_ON_COMPAT);
    let ok = write(&dir, "native-v2.ts", GOOD_V2);

    assert!(js.load(&v1).is_err());
    assert!(js.load(&v2).is_err());
    let good = js.load(&ok).unwrap();
    assert_eq!(good.api_family, ApiFamily::V2);

    let failures = js.load_failures();
    assert_eq!(failures.len(), 2, "{failures:?}");
    assert!(failures
        .iter()
        .any(|f| { f.identity_key.contains("native-v1") && f.diagnostic.contains("api-version") }));
    assert!(failures
        .iter()
        .any(|f| { f.identity_key.contains("compat-v2") && f.diagnostic.contains("api-version") }));
    assert!(failures.iter().all(|f| !f.identity_key.is_empty()));
    assert!(!failures
        .iter()
        .any(|f| f.identity_key == good.identity_key()));
}
