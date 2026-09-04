// Task 2: `$RS2B0T` registry parse. `index.ts` is parsed statically — no
// rustyscript, no V8 Runtime, no isolate. Register names are the picker
// names (they may differ from the folder); the matched import path is the
// file to read on Start. `register_rs2b0t` fills the JS library from a
// root and persists the root path on the first successful parse.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use script::load::{JsLibrary, LoadShape};
use script::{ScriptKind, ScriptSource};

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

fn test_library(dir: &Path) -> JsLibrary {
    JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache"))
}

#[test]
fn parse_registry_yields_register_name_and_import_path() {
    let cards = script::parse_registry(FAKE_INDEX).expect("fake index parses");
    assert_eq!(cards.len(), 3);
    assert_eq!(cards[0].name, "BoneBurier");
    assert_eq!(cards[0].rel_path, "./BoneBurier/BoneBurier.js");
    assert_eq!(cards[0].description, "Buries bones from the bank");
    assert_eq!(cards[0].category, "Prayer");
    assert!(cards[0].tags.is_empty());
    assert_eq!(cards[0].version, "");
    assert!(cards[0].settings_schema.is_empty());
    assert_eq!(cards[0].kind, ScriptKind::Compat);
    assert_eq!(cards[0].source, ScriptSource::Catalog);
    assert_eq!(
        cards[1].name, "AIO Teleport",
        "picker name is the register name, not the folder"
    );
    assert_eq!(cards[1].rel_path, "./AIOTeleport/AIOTeleport.js");
    assert_eq!(cards[1].description, "Automated teleportation");
    assert_eq!(
        cards[2].name, "ShopRunner",
        "named import matches the create"
    );
    assert_eq!(cards[2].rel_path, "./ShopRunner/ShopRunner.js");
    assert_eq!(cards[2].description, "Runs the shops");
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
fn rs2b0t_import_deferred_roundtrip() {
    let dir = scratch("rs2b0t_import");
    let import_file = dir.join("rs2b0t-import");
    assert!(!script::rs2b0t_import_deferred_at(&import_file));
    script::set_rs2b0t_import_deferred_at(&import_file).expect("defer");
    assert!(script::rs2b0t_import_deferred_at(&import_file));
    script::clear_rs2b0t_import_at(&import_file).expect("clear");
    assert!(!script::rs2b0t_import_deferred_at(&import_file));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rs2b0t_root_prefers_env_then_persisted_file() {
    let iso = script::IsolatedEnv::enter("rs2b0t-root");
    let path_file = iso.dir.join("rs2b0t-path");
    let env_root = iso.dir.join("env-root");
    let persisted = iso.dir.join("persisted-root");
    std::fs::create_dir_all(&env_root).unwrap();

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

    iso.set_rs2b0t(&env_root);
    assert_eq!(
        script::rs2b0t_root_at(&path_file).as_deref(),
        Some(env_root.as_path()),
        "$RS2B0T wins over the persisted path"
    );

    iso.clear_rs2b0t();
    assert_eq!(
        script::rs2b0t_root_at(&path_file).as_deref(),
        Some(persisted.as_path()),
        "env gone: the persisted fallback is back"
    );
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
    let mut lib = test_library(&dir);
    let n = lib
        .register_rs2b0t(&root, &path_file)
        .expect("registry fills");
    assert_eq!(n, 2);

    let bone = lib
        .get(ScriptSource::Catalog, "BoneBurier")
        .expect("BoneBurier card");
    assert_eq!(bone.shape, LoadShape::CompatClass);
    assert!(bone.origin.contains("extends LoopingBot"));
    assert!(
        bone.path.ends_with("BoneBurier.ts"),
        "the .js import path resolves to the .ts file on disk: {}",
        bone.path.display()
    );

    let aio = lib
        .get(ScriptSource::Catalog, "AIOQuester")
        .expect("AIOQuester card");
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

    let mut lib = test_library(&dir);
    let n = lib
        .register_rs2b0t(&root, &dir.join("rs2b0t-path"))
        .expect("registry fills");
    assert_eq!(n, 1, "WalkTo is host nav, never a JS card");

    let card = lib
        .get(ScriptSource::Catalog, "AIO Teleport")
        .expect("register name is the picker name");
    assert_eq!(card.shape, LoadShape::CompatClass);
    assert!(card.origin.contains("class T"));
    assert!(lib.get(ScriptSource::Catalog, "WalkTo").is_none());
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
    std::fs::write(
        scripts.join("BoneBurier/BoneBurier.ts"),
        "export default class {}",
    )
    .unwrap();

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

const CHICKEN_INDEX: &str = r#"
import ChickenKiller, { SETTINGS as CHICKEN_SETTINGS } from './ChickenKiller/ChickenKiller.js';

ScriptRegistry.register({
    name: 'ChickenKiller',
    description: 'Kills chickens for combat XP',
    category: 'Combat',
    tags: ['combat', 'money'],
    version: '1.2.0',
    settingsSchema: CHICKEN_SETTINGS,
    create: () => new ChickenKiller()
});
"#;

const CHICKEN_KILLER_TS: &str = r#"
export const SETTINGS = {
    leashRadius: {
        type: 'number',
        default: 12,
        label: 'Leash radius',
        min: 1,
        max: 50,
        step: 1,
        group: 'Combat',
        help: 'Max tiles from start tile',
    },
    buryBones: {
        type: 'boolean',
        default: true,
        label: 'Bury bones',
    },
    combatStyle: {
        type: 'string',
        default: 'melee',
        label: 'Combat style',
        options: ['melee', 'ranged', 'magic'],
    },
};
"#;

#[test]
fn parse_registry_resolves_settings_schema_via_import_alias() {
    let mut sources = HashMap::new();
    sources.insert(
        "./ChickenKiller/ChickenKiller.js".to_string(),
        CHICKEN_KILLER_TS.to_string(),
    );
    let cards =
        script::parse_registry_with_sources(CHICKEN_INDEX, &sources).expect("chicken index parses");
    assert_eq!(cards.len(), 1);
    let card = &cards[0];
    assert_eq!(card.name, "ChickenKiller");
    assert_eq!(card.description, "Kills chickens for combat XP");
    assert_eq!(card.category, "Combat");
    assert_eq!(card.tags, vec!["combat", "money"]);
    assert_eq!(card.version, "1.2.0");
    assert_eq!(card.kind, ScriptKind::Compat);
    assert_eq!(card.source, ScriptSource::Catalog);
    assert_eq!(card.settings_schema.len(), 3);

    let leash = &card.settings_schema[0];
    assert_eq!(leash.id, "leashRadius");
    assert_eq!(leash.ty, "number");
    assert_eq!(leash.default.as_deref(), Some("12"));
    assert_eq!(leash.label.as_deref(), Some("Leash radius"));
    assert_eq!(leash.min.as_deref(), Some("1"));
    assert_eq!(leash.max.as_deref(), Some("50"));
    assert_eq!(leash.step.as_deref(), Some("1"));
    assert_eq!(leash.group.as_deref(), Some("Combat"));
    assert_eq!(leash.help.as_deref(), Some("Max tiles from start tile"));

    let bury = &card.settings_schema[1];
    assert_eq!(bury.id, "buryBones");
    assert_eq!(bury.ty, "boolean");
    assert_eq!(bury.default.as_deref(), Some("true"));
    assert_eq!(bury.label.as_deref(), Some("Bury bones"));

    let style = &card.settings_schema[2];
    assert_eq!(style.id, "combatStyle");
    assert_eq!(style.ty, "string");
    assert_eq!(style.default.as_deref(), Some("melee"));
    assert_eq!(style.label.as_deref(), Some("Combat style"));
    assert_eq!(
        style.options,
        vec![
            "melee".to_string(),
            "ranged".to_string(),
            "magic".to_string()
        ]
    );
}

#[test]
fn parse_registry_identifier_options_does_not_execute_ts() {
    let index = r#"
import Alcher, { ALCHER_SETTINGS } from './Alcher/Alcher.js';

ScriptRegistry.register({
    name: 'Alcher',
    settingsSchema: ALCHER_SETTINGS,
    create: () => new Alcher()
});
"#;
    let alcher_ts = r#"
export const ALCHER_SETTINGS = {
    combatStyle: {
        type: 'string',
        default: 'melee',
        options: COMBAT_STYLE_OPTIONS,
    },
};
"#;
    let mut sources = HashMap::new();
    sources.insert("./Alcher/Alcher.js".to_string(), alcher_ts.to_string());
    let cards = script::parse_registry_with_sources(index, &sources).expect("alcher parses");
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].settings_schema.len(), 1);
    let setting = &cards[0].settings_schema[0];
    assert_eq!(setting.id, "combatStyle");
    assert!(
        setting.options.is_empty(),
        "identifier options are not evaluated"
    );
}

#[test]
fn parse_registry_identifier_valued_setting_inlines_export_const() {
    let index = r#"
import Thiever, { SETTINGS } from './ThievingBot/ThievingBot.js';

ScriptRegistry.register({
    name: 'Thiever',
    settingsSchema: SETTINGS,
    create: () => new Thiever()
});
"#;
    let thiever_ts = r#"
import { LOADOUT_SETTING } from '../../api/loadout/loadoutSetting.js';

export const SETTINGS = {
    target: {
        type: 'string',
        default: 'Man',
        label: 'Target',
    },
    loadout: LOADOUT_SETTING,
    eatAt: {
        type: 'number',
        default: 10,
        label: 'Eat at',
    },
};
"#;
    let mut sources = HashMap::new();
    sources.insert(
        "./ThievingBot/ThievingBot.js".to_string(),
        thiever_ts.to_string(),
    );
    let cards = script::parse_registry_with_sources(index, &sources).expect("thiever parses");
    assert_eq!(cards.len(), 1);
    assert_eq!(
        cards[0].settings_schema.len(),
        3,
        "LOADOUT_SETTING must not abort the object walk"
    );
    assert_eq!(cards[0].settings_schema[0].id, "target");
    let loadout = &cards[0].settings_schema[1];
    assert_eq!(loadout.id, "loadout");
    assert_eq!(loadout.ty, "string");
    assert_eq!(
        loadout.options_from.as_deref(),
        Some("loadouts"),
        "quoted optionsFrom on the inlined const"
    );
    assert_eq!(cards[0].settings_schema[2].id, "eatAt");
}

#[test]
fn parse_settings_skips_typescript_type_annotation() {
    let src = r#"
export const SETTINGS: SettingsSchema = {
    thieveTarget: { type: 'string', default: 'Guard', label: 'Pickpocket target' },
    guardResponse: { type: 'string', default: 'Flee', label: 'Guard response' },
    ...PERIODIC_BANK_SETTINGS
};
"#;
    let schema = script::settings_schema_from_source(src);
    let ids: Vec<&str> = schema.iter().map(|s| s.id.as_str()).collect();
    assert!(
        ids.contains(&"thieveTarget"),
        "ArdyThiever-shaped SETTINGS: Type annotation must not empty the schema: {ids:?}"
    );
    assert!(
        ids.contains(&"guardResponse"),
        "fields after thieveTarget must survive: {ids:?}"
    );
    assert!(
        ids.contains(&"bankStrategy"),
        "...PERIODIC_BANK_SETTINGS after a typed SETTINGS must still inline: {ids:?}"
    );
}

#[test]
fn parse_settings_inlines_same_file_and_shim_spreads() {
    let src = r#"
export const EXTRA = {
    panicHp: { type: 'number', default: 25, label: 'Panic HP' },
};
export const SETTINGS = {
    target: { type: 'string', default: 'Man', label: 'Target' },
    ...EXTRA,
    ...PERIODIC_BANK_SETTINGS,
};
"#;
    let schema = script::settings_schema_from_source(src);
    let ids: Vec<&str> = schema.iter().map(|s| s.id.as_str()).collect();
    assert!(
        ids.contains(&"target"),
        "own fields survive a spread: {ids:?}"
    );
    assert!(
        ids.contains(&"panicHp"),
        "same-file ...EXTRA must inline: {ids:?}"
    );
    assert!(
        ids.contains(&"bankStrategy"),
        "...PERIODIC_BANK_SETTINGS must inline the banking shim: {ids:?}"
    );
    assert!(
        ids.contains(&"bankEveryItems"),
        "spread must not drop fields after PERIODIC_BANK_SETTINGS: {ids:?}"
    );
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

    let mut lib = test_library(&dir);
    let n = lib
        .register_rs2b0t(&root, &dir.join("rs2b0t-path"))
        .expect("registry fills");
    assert_eq!(n, 1, "escaped path must not register as a card");
    assert!(lib.get(ScriptSource::Catalog, "Outside").is_none());
    assert!(lib.get(ScriptSource::Catalog, "BoneBurier").is_some());
}
