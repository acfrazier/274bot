//! Registration routing through the real File Load and catalog preparation:
//! a native entry needs a manifest, a recognized compat script loads as
//! before, and an unrecognized default export stays `NotBotShape`.
//!
//! Design: `SCRIPT-MANIFEST-DESIGN-1022/design-final2.md` section 3.2 and the
//! "Revision B1 classification regressions".

use std::path::{Path, PathBuf};
use std::sync::Arc;

use script::preparation::CooperativePreparationStep;
use script_js::{ApiFamily, CatalogDiff, JsLibrary, LoadShape};

const MANIFEST_TS: &str = r#"import type { NativeApi, ScriptManifest } from '../host-js/index.d.ts';

export const manifest = {
    apiVersion: 2,
    name: 'BoneBurier',
    version: '1.0.0',
    author: 'alice',
    description: 'Bury bones and restock from a supported bank.',
    category: 'Prayer',
    tags: ['prayer', 'banking'],
    settings: {
        boneName: { type: 'string', default: 'Bones', label: 'Bone name', help: 'The item to bury' },
    },
    capabilities: [],
} as const satisfies ScriptManifest;

export async function tick(api: NativeApi): Promise<void> {
    api.log(api.settings.str('boneName', 'Bones'));
}
"#;

const UNVERSIONED_TICK: &str = "export function tick(api) { api.log('x'); }\n";
const STANDALONE_V2_TICK: &str =
    "export const apiVersion = 2;\nexport async function tick(api) { api.log('x'); }\n";
const COMPAT_TEXT_WITHOUT_DEFAULT: &str = "// defineBot(\n\
     const marker = '__rs2b0tManifest';\n\
     export function tick(api) { api.log(marker); }\n";
const DEFINE_BOT_JS: &str = "import { defineBot } from '@rs2b0t/api';\n\
     // A comment and CRLF-free bytes stay verbatim.\n\
     export default defineBot({ name: 'Compat', create: () => ({ loop() {} }) });\n";
const COMPAT_CLASS_TS: &str =
    "export default class Herbs extends LoopingBot { private n: number = 0; loop() {} }\n";
const MIGRATION_EXAMPLE: &str = "export const manifest = { apiVersion: 2, name: 'My script', version: '1', author: 'Your name', description: 'What it does', category: 'Examples', tags: [], settings: {}, capabilities: [] };";

// The frozen rs2b0t `90a69240a` shapes: `ChickenKiller/CowKiller.ts:52`,
// `GatheringBot/Woodcutter.ts:70` and
// `EdgevilleMonkeyBars/EdgevilleMonkeyBars.ts:249,591`.
const CHICKEN_KILLER: &str = "export const SETTINGS = {};\n\
     export default class ChickenKiller extends LoopingBot { loop() {} }\n";
const COW_KILLER: &str =
    "import ChickenKiller, { SETTINGS as CHICKEN_SETTINGS } from './ChickenKiller.js';\n\
     export const SETTINGS = { ...CHICKEN_SETTINGS };\n\
     export default class CowKiller extends ChickenKiller {}\n";
const GATHERING_BOT: &str = "export const GATHERING_SETTINGS = {};\n\
     export default class GatheringBot extends LoopingBot { loop() {} }\n";
const WOODCUTTER: &str = "import GatheringBot, { GATHERING_SETTINGS } from './GatheringBot.js';\n\
     export const WOODCUTTER_SETTINGS = { ...GATHERING_SETTINGS };\n\
     export default class Woodcutter extends GatheringBot {}\n";
const EDGEVILLE_MONKEY_BARS: &str = "import { TaskBot } from '../../api/bot/Bot.js';\n\
     class EdgevilleMonkeyBars extends TaskBot {}\n\
     export default EdgevilleMonkeyBars;\n";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("274bot-load-shape-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create load-shape scratch directory");
    dir
}

fn library(dir: &Path) -> JsLibrary {
    JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache"))
}

fn write(path: &Path, source: &str) -> PathBuf {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create fixture directory");
    }
    std::fs::write(path, source).expect("write fixture source");
    path.to_path_buf()
}

#[test]
fn manifest_file_loads_as_checked_v2_with_manifest_metadata() {
    let dir = scratch("manifest");
    let path = write(&dir.join("bone_burier.ts"), MANIFEST_TS);
    let mut lib = library(&dir);
    let card = lib.load(&path).expect("manifest-form File Load");
    assert_eq!(card.shape(), LoadShape::NativeTick);
    assert_eq!(card.api_family(), ApiFamily::V2);
    assert!(card.checked.v2_native());
    assert_eq!(
        card.name, "bone_burier",
        "File identity name stays the stem"
    );
    assert_eq!(
        card.description,
        "Bury bones and restock from a supported bank."
    );
    assert_eq!(card.category, "Prayer");
    assert_eq!(card.tags, ["prayer".to_owned(), "banking".to_owned()]);
    assert_eq!(card.settings_schema.len(), 1);
    assert_eq!(card.settings_schema[0].id, "boneName");
    assert_eq!(
        card.settings_schema[0].help.as_deref(),
        Some("The item to bury")
    );
    assert!(!card.js.contains("manifest"), "execution JS: {}", card.js);
    assert!(!card.js.contains("apiVersion"), "execution JS: {}", card.js);
    assert!(
        card.js.contains("function tick"),
        "execution JS: {}",
        card.js
    );
    assert_eq!(
        card.manifest.as_ref().map(|manifest| manifest.name()),
        Some("BoneBurier")
    );
}

#[test]
fn native_file_without_manifest_is_manifest_required() {
    let dir = scratch("required");
    for (file, source) in [
        ("unversioned.js", UNVERSIONED_TICK),
        ("unversioned_ts.ts", UNVERSIONED_TICK),
        ("standalone_v2.ts", STANDALONE_V2_TICK),
        ("compat_text.js", COMPAT_TEXT_WITHOUT_DEFAULT),
    ] {
        let path = write(&dir.join(file), source);
        let mut lib = library(&dir);
        let error = lib
            .load(&path)
            .expect_err("no-manifest native source refuses");
        assert!(
            error.contains("manifest-required at manifest"),
            "{file}: {error}"
        );
        assert!(error.contains(MIGRATION_EXAMPLE), "{file}: {error}");
        assert!(
            error.contains(&format!("{}:", path.display())),
            "{file} names its source path: {error}"
        );
        assert!(lib.cards().is_empty(), "{file} registers no card");
        let failures = lib.load_failures();
        assert_eq!(failures.len(), 1, "{file}");
        assert!(
            failures[0].diagnostic.contains("manifest-required"),
            "{file}: {}",
            failures[0].diagnostic
        );
        assert_eq!(failures[0].api_family, None, "{file}: no invented family");
    }
}

#[test]
fn recognized_compat_files_load_as_before() {
    let dir = scratch("compat");
    let define_bot = write(&dir.join("compat.js"), DEFINE_BOT_JS);
    let class = write(&dir.join("herbs.ts"), COMPAT_CLASS_TS);
    let mut lib = library(&dir);
    let card = lib.load(&define_bot).expect("defineBot File Load");
    assert_eq!(card.shape(), LoadShape::CompatDefineBot);
    assert_eq!(card.api_family(), ApiFamily::V1);
    assert_eq!(card.js, DEFINE_BOT_JS, "compat JS executes byte-for-byte");
    assert!(card.manifest.is_none());
    let card = lib.load(&class).expect("compat class File Load");
    assert_eq!(card.shape(), LoadShape::CompatClass);
    assert_eq!(card.api_family(), ApiFamily::V1);
}

/// Parent-passing control: these frozen shapes were `Reject` before the cut
/// and stay `NotBotShape`, never a migration diagnostic.
#[test]
fn unrecognized_rs2b0t_defaults_keep_not_bot_shape_on_file_load() {
    let dir = scratch("frozen-file");
    write(&dir.join("ChickenKiller/ChickenKiller.ts"), CHICKEN_KILLER);
    write(&dir.join("GatheringBot/GatheringBot.ts"), GATHERING_BOT);
    let cases = [
        write(&dir.join("ChickenKiller/CowKiller.ts"), COW_KILLER),
        write(&dir.join("GatheringBot/Woodcutter.ts"), WOODCUTTER),
        write(
            &dir.join("EdgevilleMonkeyBars/EdgevilleMonkeyBars.ts"),
            EDGEVILLE_MONKEY_BARS,
        ),
    ];
    for path in cases {
        let mut lib = library(&dir);
        let stem = path.file_stem().unwrap().to_str().unwrap().to_owned();
        let error = lib.load(&path).expect_err("unrecognized default export");
        assert_eq!(error, format!("not a bot shape: {stem}"));
        assert!(lib.cards().is_empty());
    }
}

/// New refusal: compat markers without a runtime default export could never
/// Start (the wrapper reads `mod.default`), and now refuse at Load.
#[test]
fn compat_markers_without_default_are_not_bot_shapes_on_file_load() {
    let dir = scratch("markers");
    for (file, source) in [
        ("define_bot.js", "defineBot({ name: 'A' });\n"),
        (
            "marker.js",
            "export const __rs2b0tManifest = { api: '1.0' };\n",
        ),
    ] {
        let path = write(&dir.join(file), source);
        let mut lib = library(&dir);
        let error = lib.load(&path).expect_err("marker without default export");
        assert!(error.starts_with("not a bot shape:"), "{file}: {error}");
    }
}

/// The pre-cut cache kept one `objects/<origin-sha>.js` per origin. A stale
/// unerased object there is never served to a manifest-form entry.
#[test]
fn old_cache_object_is_not_an_entry_hit() {
    let dir = scratch("old-cache");
    let path = write(&dir.join("bone_burier.ts"), MANIFEST_TS);
    let old_objects = dir.join("js-cache").join("objects");
    std::fs::create_dir_all(&old_objects).unwrap();
    let sha = script_js::JsCache::origin_sha(MANIFEST_TS.as_bytes());
    std::fs::write(
        old_objects.join(format!("{sha}.js")),
        "export const manifest = { stale: true };\nexport function tick() {}\n",
    )
    .unwrap();
    let mut lib = library(&dir);
    let card = lib.load(&path).expect("manifest-form File Load");
    assert!(
        !card.js.contains("stale"),
        "stale object served: {}",
        card.js
    );
    assert!(!card.js.contains("manifest"), "execution JS: {}", card.js);
}

fn write_frozen_catalog(root: &Path) {
    let scripts = root.join("src/bot/scripts");
    write(
        &scripts.join("ChickenKiller/ChickenKiller.ts"),
        CHICKEN_KILLER,
    );
    write(&scripts.join("ChickenKiller/CowKiller.ts"), COW_KILLER);
    write(&scripts.join("GatheringBot/GatheringBot.ts"), GATHERING_BOT);
    write(&scripts.join("GatheringBot/Woodcutter.ts"), WOODCUTTER);
    write(
        &scripts.join("EdgevilleMonkeyBars/EdgevilleMonkeyBars.ts"),
        EDGEVILLE_MONKEY_BARS,
    );
    write(
        &scripts.join("NativeNoManifest/NativeNoManifest.ts"),
        STANDALONE_V2_TICK,
    );
    write(
        &scripts.join("index.ts"),
        r#"
import ChickenKiller from './ChickenKiller/ChickenKiller.js';
import CowKiller from './ChickenKiller/CowKiller.js';
import GatheringBot from './GatheringBot/GatheringBot.js';
import Woodcutter from './GatheringBot/Woodcutter.js';
import EdgevilleMonkeyBars from './EdgevilleMonkeyBars/EdgevilleMonkeyBars.js';
import NativeNoManifest from './NativeNoManifest/NativeNoManifest.js';
ScriptRegistry.register({ name: 'ChickenKiller', create: () => new ChickenKiller() });
ScriptRegistry.register({ name: 'CowKiller', create: () => new CowKiller() });
ScriptRegistry.register({ name: 'GatheringBot', create: () => new GatheringBot() });
ScriptRegistry.register({ name: 'Woodcutter', create: () => new Woodcutter() });
ScriptRegistry.register({ name: 'EdgevilleMonkeyBars', create: () => new EdgevilleMonkeyBars() });
ScriptRegistry.register({ name: 'NativeNoManifest', create: () => new NativeNoManifest() });
"#,
    );
}

const FROZEN: [&str; 3] = ["CowKiller", "Woodcutter", "EdgevilleMonkeyBars"];

#[test]
fn synchronous_catalog_skips_frozen_shapes_and_refuses_no_manifest_native() {
    let dir = scratch("catalog-sync");
    let root = dir.join("rs2b0t");
    write_frozen_catalog(&root);
    let mut lib = library(&dir);
    lib.register_rs2b0t(&root, &dir.join("rs2b0t-path"))
        .expect("catalog registers");
    let names: Vec<&str> = lib.cards().iter().map(|card| card.name.as_str()).collect();
    assert!(names.contains(&"ChickenKiller"), "{names:?}");
    assert!(names.contains(&"GatheringBot"), "{names:?}");
    for card in lib.cards() {
        assert_eq!(card.api_family(), ApiFamily::V1, "{}", card.name);
    }
    for name in FROZEN.into_iter().chain(["NativeNoManifest"]) {
        assert!(!names.contains(&name), "{name} registered: {names:?}");
    }
    let failures = lib.load_failures();
    let failed: Vec<&str> = failures
        .iter()
        .map(|failure| failure.name.as_str())
        .collect();
    assert_eq!(failed, ["NativeNoManifest"], "{failures:?}");
    assert!(
        failures[0].diagnostic.contains("manifest-required"),
        "{}",
        failures[0].diagnostic
    );
}

fn incremental_diff(lib: &JsLibrary, root: &Path) -> CatalogDiff {
    let snapshot = Arc::new(
        lib.catalog_preparation_snapshot(root)
            .expect("catalog preparation snapshot"),
    );
    api::selected::FamilyPreparation::run(move |worker| {
        let mut work = snapshot.cooperative_diff();
        loop {
            match work(worker) {
                CooperativePreparationStep::Continue { .. } => {}
                CooperativePreparationStep::Complete { output, .. } => {
                    break output.expect("catalog diff completes");
                }
            }
        }
    })
    .expect("spawn preparation thread")
    .join()
    .expect("preparation thread")
}

#[test]
fn incremental_catalog_skips_frozen_shapes_and_refuses_no_manifest_native() {
    let dir = scratch("catalog-incremental");
    let root = dir.join("rs2b0t");
    write_frozen_catalog(&root);
    let lib = library(&dir);
    let diff = incremental_diff(&lib, &root);
    assert!(diff.added.contains(&"ChickenKiller".to_owned()), "{diff:?}");
    assert!(diff.added.contains(&"GatheringBot".to_owned()), "{diff:?}");
    for name in FROZEN.into_iter().chain(["NativeNoManifest"]) {
        assert!(
            !diff.added.iter().any(|added| added == name),
            "{name}: {diff:?}"
        );
    }
    let failed: Vec<&str> = diff
        .failed
        .iter()
        .map(|failure| failure.name.as_str())
        .collect();
    assert_eq!(failed, ["NativeNoManifest"], "{diff:?}");
    assert!(
        diff.failed[0].diagnostic.contains("manifest-required"),
        "{}",
        diff.failed[0].diagnostic
    );
}
