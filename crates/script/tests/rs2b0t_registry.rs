// Task 2: `$RS2B0T` registry parse. `index.ts` is parsed statically — no
// rustyscript, no V8 Runtime, no isolate. Register names are the picker
// names (they may differ from the folder); the matched import path is the
// file to read on Start. `register_rs2b0t` fills the JS library from a
// root and persists the root path on the first successful parse.

use std::path::{Path, PathBuf};

use script::load::{JsLibrary, LoadShape};

/// The brief's fake `index.ts`: a default import, a display name that
/// differs from the folder (`AIO Teleport` vs `AIOTeleport`), a
/// multi-line default import, and a named import (`ShopRunner`).
const FAKE_INDEX: &str = r#"
import { ScriptRegistry } from '../runtime/ScriptRegistry.js';
import BoneBurier, { BONE_BURIER_SETTINGS } from './BoneBurier/BoneBurier.js';
import AIOTeleport, {
    SETTINGS as AIOTELEPORT_SETTINGS
} from './AIOTeleport/AIOTeleport.js';
import { ShopRunner, SHOPRUNNER_SETTINGS } from './ShopRunner/ShopRunner.js';

ScriptRegistry.register({
    name: 'BoneBurier',
    description: 'Buries bones from the bank',
    category: 'Prayer',
    create: () => new BoneBurier()
});

ScriptRegistry.register({
    name: 'AIO Teleport',
    description: 'Automated teleportation',
    create: () => new AIOTeleport()
});

ScriptRegistry.register({
    name: 'ShopRunner',
    description: 'Runs the shops',
    create: () => new ShopRunner()
});
"#;

/// Unique temp dir per test binary (existing 274bot convention), plus a
/// per-test scratch subdir so parallel tests never share files.
fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("274bot-rs2b0t-registry-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn scratch(name: &str) -> PathBuf {
    let dir = temp_dir().join(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn parse_registry_yields_register_name_and_import_path() {
    let cards = script::parse_registry(FAKE_INDEX).expect("fake index parses");
    assert_eq!(cards.len(), 3);
    assert_eq!(cards[0].name, "BoneBurier");
    assert_eq!(cards[0].rel_path, "./BoneBurier/BoneBurier.js");
    assert_eq!(
        cards[1].name, "AIO Teleport",
        "picker name is the register name, not the folder"
    );
    assert_eq!(cards[1].rel_path, "./AIOTeleport/AIOTeleport.js");
    assert_eq!(
        cards[2].name, "ShopRunner",
        "named import matches the create"
    );
    assert_eq!(cards[2].rel_path, "./ShopRunner/ShopRunner.js");
}

#[test]
fn parse_registry_errors_on_non_registry_index() {
    assert!(
        script::parse_registry("const x = 1;").is_err(),
        "a file with no ScriptRegistry.register is not a registry index"
    );
    assert!(script::parse_registry("").is_err());
    assert!(
        script::parse_registry("ScriptRegistry.register({ name: 'X' });").is_err(),
        "a register whose create matches no import cannot yield a card"
    );
}

#[test]
fn rs2b0t_root_prefers_env_then_persisted_file() {
    let dir = scratch("rs2b0t_root");
    let path_file = dir.join("rs2b0t-path");
    let env_root = dir.join("env-root");
    let persisted = dir.join("persisted-root");
    std::fs::create_dir_all(&env_root).unwrap();

    // Env is restored afterwards; no other script test reads RS2B0T.
    let orig = std::env::var("RS2B0T").ok();
    std::env::remove_var("RS2B0T");

    assert_eq!(
        script::rs2b0t_root_at(&path_file),
        None,
        "no env, no persisted path: no root"
    );

    script::persist_rs2b0t_root_at(&persisted, &path_file).expect("persists the path");
    assert_eq!(
        script::rs2b0t_root_at(&path_file).as_deref(),
        Some(persisted.as_path()),
        "the persisted path is the fallback"
    );

    std::env::set_var("RS2B0T", &env_root);
    assert_eq!(
        script::rs2b0t_root_at(&path_file).as_deref(),
        Some(env_root.as_path()),
        "$RS2B0T wins over the persisted path"
    );

    std::env::remove_var("RS2B0T");
    assert_eq!(
        script::rs2b0t_root_at(&path_file).as_deref(),
        Some(persisted.as_path()),
        "env gone: the persisted fallback is back"
    );

    match orig {
        Some(v) => std::env::set_var("RS2B0T", v),
        None => std::env::remove_var("RS2B0T"),
    }
}

#[test]
fn js_library_registers_rs2b0t_cards_without_isolates() {
    let dir = scratch("rs2b0t_fill");
    let root = dir.join("rs2b0t");
    let scripts = root.join("src/bot/scripts");
    std::fs::create_dir_all(scripts.join("BoneBurier")).unwrap();
    std::fs::create_dir_all(scripts.join("AIOQuester")).unwrap();
    std::fs::write(
        scripts.join("index.ts"),
        r#"
import BoneBurier from './BoneBurier/BoneBurier.js';
import AIOQuester from './AIOQuester/AIOQuester.js';

ScriptRegistry.register({ name: 'BoneBurier', create: () => new BoneBurier() });
ScriptRegistry.register({ name: 'AIOQuester', create: () => new AIOQuester() });
"#,
    )
    .unwrap();
    // The import says `.js` but the file on disk is `.ts`.
    std::fs::write(
        scripts.join("BoneBurier/BoneBurier.ts"),
        "export default class BoneBurier extends LoopingBot { override loop() {} }",
    )
    .unwrap();
    std::fs::write(
        scripts.join("AIOQuester/AIOQuester.js"),
        "export function tick(api) { api._n = (api._n||0)+1 }",
    )
    .unwrap();

    let path_file = dir.join("rs2b0t-path");
    let mut lib = JsLibrary::new(dir.join("js-scripts.json"));
    let n = lib
        .register_rs2b0t(&root, &path_file)
        .expect("registry fills");
    assert_eq!(n, 2);

    let bone = lib.get("BoneBurier").expect("BoneBurier card");
    assert_eq!(bone.shape, LoadShape::CompatClass);
    assert!(bone.source.contains("extends LoopingBot"));
    assert!(
        bone.path.ends_with("BoneBurier.ts"),
        "the .js import path resolves to the .ts file on disk: {}",
        bone.path.display()
    );

    let aio = lib.get("AIOQuester").expect("AIOQuester card");
    assert_eq!(aio.shape, LoadShape::NativeTick);

    // The first successful parse persisted the root path.
    assert_eq!(
        script::rs2b0t_root_at(&path_file).as_deref(),
        Some(root.as_path())
    );
}

#[test]
fn rs2b0t_fill_uses_register_name_and_skips_reserved_walk_to() {
    let dir = scratch("rs2b0t_names");
    let root = dir.join("rs2b0t");
    let scripts = root.join("src/bot/scripts");
    std::fs::create_dir_all(scripts.join("AIOTeleport")).unwrap();
    std::fs::create_dir_all(scripts.join("WalkToBot")).unwrap();
    std::fs::write(
        scripts.join("index.ts"),
        r#"
import AIOTeleport from './AIOTeleport/AIOTeleport.js';
import WalkToBot from './WalkToBot/WalkToBot.js';

ScriptRegistry.register({ name: 'AIO Teleport', create: () => new AIOTeleport() });
ScriptRegistry.register({ name: 'WalkTo', create: () => new WalkToBot() });
"#,
    )
    .unwrap();
    std::fs::write(
        scripts.join("AIOTeleport/AIOTeleport.ts"),
        "export default class T extends LoopingBot { override loop() {} }",
    )
    .unwrap();
    std::fs::write(
        scripts.join("WalkToBot/WalkToBot.ts"),
        "export default class W extends LoopingBot { override loop() {} }",
    )
    .unwrap();

    let mut lib = JsLibrary::new(dir.join("js-scripts.json"));
    let n = lib
        .register_rs2b0t(&root, &dir.join("rs2b0t-path"))
        .expect("registry fills");
    assert_eq!(n, 1, "WalkTo is host nav, never a JS card");

    let card = lib
        .get("AIO Teleport")
        .expect("register name is the picker name");
    assert_eq!(card.shape, LoadShape::CompatClass);
    assert!(card.source.contains("class T"));
    assert!(lib.get("WalkTo").is_none());
}

#[test]
fn parse_registry_is_relative_path_to_catalog_dir() {
    let cards = script::parse_registry(FAKE_INDEX).unwrap();
    let root = Path::new("/tmp/rs2b0t");
    let p = script::script_file_path(root, &cards[0].rel_path).expect("valid catalog path");
    assert_eq!(p, root.join("src/bot/scripts/BoneBurier/BoneBurier.js"));
}

#[test]
fn script_file_path_rejects_escape_and_absolute() {
    let root = Path::new("/tmp/rs2b0t");
    assert!(
        script::script_file_path(root, "../../etc/passwd").is_none(),
        "../../etc/passwd must not resolve outside the catalog"
    );
    assert!(
        script::script_file_path(root, "../evil.ts").is_none(),
        "../evil.ts must not resolve outside the catalog"
    );
    assert!(
        script::script_file_path(root, "./../../etc/passwd").is_none(),
        "./../../etc/passwd must not escape via a ./ prefix"
    );
    assert!(
        script::script_file_path(root, "/etc/passwd").is_none(),
        "absolute imports must be rejected"
    );
}

#[test]
fn script_file_path_resolves_js_import_to_ts_twin() {
    let dir = scratch("script_path_ts_twin");
    let root = dir.join("rs2b0t");
    let scripts = root.join("src/bot/scripts");
    std::fs::create_dir_all(scripts.join("BoneBurier")).unwrap();
    std::fs::write(scripts.join("BoneBurier/BoneBurier.ts"), "export default class {}").unwrap();

    let p = script::script_file_path(&root, "./BoneBurier/BoneBurier.js")
        .expect("valid ./ import resolves");
    let expected = scripts.join("BoneBurier/BoneBurier.ts");
    assert_eq!(
        p.canonicalize().expect("resolved path exists"),
        expected.canonicalize().expect("expected path exists"),
        ".js import path resolves to the .ts twin on disk"
    );
}

#[test]
fn parse_registry_skips_parent_dir_imports() {
    let index = r#"
import Evil from '../evil/Evil.js';
import BoneBurier from './BoneBurier/BoneBurier.js';

ScriptRegistry.register({ name: 'Evil', create: () => new Evil() });
ScriptRegistry.register({ name: 'BoneBurier', create: () => new BoneBurier() });
"#;
    let cards = script::parse_registry(index).expect("BoneBurier still registers");
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].name, "BoneBurier");
    assert_eq!(cards[0].rel_path, "./BoneBurier/BoneBurier.js");
}

#[test]
fn register_rs2b0t_skips_cards_whose_path_escapes_catalog() {
    let dir = scratch("rs2b0t_escape");
    let root = dir.join("rs2b0t");
    let scripts = root.join("src/bot/scripts");
    std::fs::create_dir_all(scripts.join("BoneBurier")).unwrap();
    // Place a readable file outside the catalog that a tampered import could target.
    std::fs::write(dir.join("outside.ts"), "export default class Outside {}").unwrap();
    std::fs::write(
        scripts.join("index.ts"),
        r#"
import Outside from '../outside.ts';
import BoneBurier from './BoneBurier/BoneBurier.js';

ScriptRegistry.register({ name: 'Outside', create: () => new Outside() });
ScriptRegistry.register({ name: 'BoneBurier', create: () => new BoneBurier() });
"#,
    )
    .unwrap();
    std::fs::write(
        scripts.join("BoneBurier/BoneBurier.ts"),
        "export default class BoneBurier extends LoopingBot { override loop() {} }",
    )
    .unwrap();

    let mut lib = JsLibrary::new(dir.join("js-scripts.json"));
    let n = lib
        .register_rs2b0t(&root, &dir.join("rs2b0t-path"))
        .expect("registry fills");
    assert_eq!(n, 1, "escaped path must not register as a card");
    assert!(lib.get("Outside").is_none());
    assert!(lib.get("BoneBurier").is_some());
}
